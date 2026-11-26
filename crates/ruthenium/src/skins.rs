//! Loads the skins the renderer asks for: from the skin cache, the texture server, or the
//! resource packs, on threads of its own. What cannot be had now is tried again later.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ruthenium_assets::{Id, Image, Pack, Resources};
use ruthenium_auth::http::{Http, UreqHttp};
use ruthenium_auth::skin;
use ruthenium_auth::textures::{PropertyKeys, Textures, profile_by_id, profile_by_name};
use ruthenium_protocol::profile::Lookup;
use ruthenium_render::SkyImage;
use ruthenium_render::block_entity::skins::LoadedSkin;
use ruthenium_sim::avatar::SkinSource;

/// Threads that load; each waits for the network.
const WORKERS: usize = 3;
/// How long after a failure to try again.
const RETRIES: [Duration; 3] = [
    Duration::from_secs(30),
    Duration::from_secs(120),
    Duration::from_secs(600),
];
/// The largest picture that is decoded, on a side.
const MAX_SIDE: u32 = 1024;

/// A skin to load: when, and how often it has been tried.
#[derive(Debug)]
struct Task {
    due: Instant,
    tried: usize,
    source: Arc<SkinSource>,
}

#[derive(Debug, Default)]
struct Queue {
    tasks: VecDeque<Task>,
    closed: bool,
}

#[derive(Debug)]
struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
    game: Option<PathBuf>,
    cache: Option<PathBuf>,
    resources: OnceLock<Option<Resources>>,
    keys: Mutex<Option<PropertyKeys>>,
}

type Answer = (Arc<SkinSource>, LoadedSkin);

/// See the module docs.
#[derive(Debug)]
pub struct SkinLoader {
    shared: Arc<Shared>,
    answers: Receiver<Answer>,
}

impl SkinLoader {
    /// `game`: the imported game files; `cache`: where skins are kept. The threads start
    /// with it and end once it is dropped and what they wait for is done.
    pub fn new(game: Option<PathBuf>, cache: Option<PathBuf>) -> SkinLoader {
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            changed: Condvar::new(),
            game,
            cache,
            resources: OnceLock::new(),
            keys: Mutex::default(),
        });
        let (to, answers) = channel();
        for i in 0..WORKERS {
            let (shared, to) = (shared.clone(), to.clone());
            let started = std::thread::Builder::new()
                .name(format!("skins-{i}"))
                .spawn(move || work(&shared, &UreqHttp::new(), &to));
            if let Err(e) = started {
                eprintln!("[ruthenium] a thread for skins did not start: {e}");
            }
        }
        SkinLoader { shared, answers }
    }

    /// Asks for a skin.
    pub fn request(&self, source: Arc<SkinSource>) {
        let mut q = self.shared.queue.lock().expect("the skins' queue");
        q.tasks.push_back(Task {
            due: Instant::now(),
            tried: 0,
            source,
        });
        self.shared.changed.notify_one();
    }

    /// A skin that has been loaded since.
    pub fn take(&self) -> Option<Answer> {
        self.answers.try_recv().ok()
    }
}

impl Drop for SkinLoader {
    fn drop(&mut self) {
        if let Ok(mut q) = self.shared.queue.lock() {
            q.closed = true;
            q.tasks.clear();
        }
        self.shared.changed.notify_all();
    }
}

/// The next task that is due; `None` once there will be none.
fn next(shared: &Shared) -> Option<Task> {
    let mut q = shared.queue.lock().ok()?;
    loop {
        if q.closed {
            return None;
        }
        let now = Instant::now();
        if let Some(i) = q.tasks.iter().position(|t| t.due <= now) {
            return q.tasks.remove(i);
        }
        let wait = q.tasks.iter().map(|t| t.due - now).min();
        let wait = wait.unwrap_or(Duration::from_secs(3600));
        q = shared.changed.wait_timeout(q, wait).ok()?.0;
    }
}

fn work(shared: &Shared, http: &dyn Http, to: &Sender<Answer>) {
    while let Some(task) = next(shared) {
        let found = load(shared, http, &task.source);
        if !found.whole
            && let Some(after) = RETRIES.get(task.tried)
            && let Ok(mut q) = shared.queue.lock()
            && !q.closed
        {
            q.tasks.push_back(Task {
                due: Instant::now() + *after,
                tried: task.tried + 1,
                source: task.source.clone(),
            });
        }
        if to.send((task.source, found.skin)).is_err() {
            return;
        }
    }
}

/// What was found, and whether that is all there is.
struct Found {
    skin: LoadedSkin,
    whole: bool,
}

fn load(shared: &Shared, http: &dyn Http, source: &SkinSource) -> Found {
    let mut found = Found {
        skin: LoadedSkin::default(),
        whole: true,
    };
    match source {
        SkinSource::Player { textures, .. } => {
            // Only what the services signed.
            if let Some(p) = textures
                && let Some(signature) = &p.signature
            {
                match signed(shared, http, &p.value, signature) {
                    Some(true) => wear(shared, &p.value, &mut found),
                    Some(false) => {}
                    None => found.whole = false,
                }
            }
        }
        SkinSource::Profile(p) => {
            match p.lookup() {
                Some(lookup) => {
                    let profile = match lookup {
                        Lookup::Name(name) => profile_by_name(http, &name),
                        Lookup::Id(id) => profile_by_id(http, id),
                    };
                    // There may be none: it is not asked for again.
                    if let Some(profile) = profile {
                        found.skin.id = Some(profile.id);
                        if let Some(value) = profile.textures {
                            wear(shared, &value, &mut found);
                        }
                    }
                }
                None => {
                    if let Some(textures) = p.textures() {
                        wear(shared, &textures.value, &mut found);
                    }
                }
            }
            // The packs' pictures go over what was found.
            if let Some(body) = p.patch.body.as_deref().and_then(|id| picture(shared, id)) {
                found.skin.body = Some(body);
            }
            if let Some(cape) = p.patch.cape.as_deref().and_then(|id| picture(shared, id)) {
                found.skin.cape = Some(cape);
            }
            if p.patch.slim.is_some() {
                found.skin.slim = p.patch.slim;
            }
        }
    }
    found
}

/// Whether the services signed this; `None` if their keys cannot be had.
fn signed(shared: &Shared, http: &dyn Http, value: &str, signature: &str) -> Option<bool> {
    let mut keys = shared.keys.lock().ok()?;
    if keys.is_none() {
        *keys = PropertyKeys::fetch(http);
    }
    Some(keys.as_ref()?.signed(value, signature))
}

/// Puts on what a textures property says.
fn wear(shared: &Shared, value: &str, found: &mut Found) {
    let t = Textures::unpack(value);
    if let Some(url) = &t.skin {
        match download(shared, url) {
            Some(png) => {
                let skin = decode(&png, 64)
                    .as_ref()
                    .and_then(ruthenium_assets::skin::process);
                found.skin.slim = skin.is_some().then_some(t.slim);
                found.skin.body = skin.as_ref().map(sky_image);
            }
            None => found.whole = false,
        }
    }
    // As the game: a skin that cannot be had leaves the cape off too.
    if let Some(url) = &t.cape
        && (t.skin.is_none() || found.skin.body.is_some())
    {
        match download(shared, url) {
            Some(png) => found.skin.cape = decode(&png, MAX_SIDE).as_ref().map(sky_image),
            None => found.whole = false,
        }
    }
}

/// A picture from the cache, or the texture server (and then it is kept).
fn download(shared: &Shared, url: &str) -> Option<Vec<u8>> {
    let kept = shared
        .cache
        .as_deref()
        .and_then(|c| skin::cache_path(c, url));
    if let Some(png) = kept.as_deref().and_then(|p| std::fs::read(p).ok()) {
        return Some(png);
    }
    match skin::download(url) {
        Ok(png) => {
            if let Some(path) = kept
                && decode(&png, MAX_SIDE).is_some()
                && let Err(e) = keep(&path, &png)
            {
                eprintln!("[ruthenium] couldn't keep a skin: {e}");
            }
            Some(png)
        }
        Err(e) => {
            eprintln!("[ruthenium] couldn't fetch a skin: {e}");
            None
        }
    }
}

fn keep(path: &Path, png: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Two threads may keep the same: each in a file of its own until it is whole.
    let part = path.with_extension(format!("{:?}.part", std::thread::current().id()));
    std::fs::write(&part, png)?;
    std::fs::rename(&part, path)
}

/// Decodes a PNG no larger than `most` on a side.
fn decode(png: &[u8], most: u32) -> Option<Image> {
    let side = |at: usize| Some(u32::from_be_bytes(png.get(at..at + 4)?.try_into().ok()?));
    let (width, height) = (side(16)?, side(20)?);
    if width == 0 || height == 0 || width > most || height > most {
        return None;
    }
    Image::decode_png(png).ok()
}

/// A picture of the resource packs, by its id.
fn picture(shared: &Shared, id: &str) -> Option<SkyImage> {
    let resources = shared
        .resources
        .get_or_init(|| open(shared.game.as_deref()?))
        .as_ref()?;
    let id = Id::parse(id)?;
    let file = id.wrap("textures/", ".png");
    let png = resources.get(&file)?.read().ok()?;
    decode(&png, MAX_SIDE).as_ref().map(sky_image)
}

/// The vanilla pack of the game imported in `game`.
pub(crate) fn open(game: &Path) -> Option<Resources> {
    let imported = ruthenium_prepare::import::open(ruthenium_prepare::supported_version(), game)?;
    let pack = Pack::vanilla(
        &imported.client_jar,
        &imported.asset_index(),
        &imported.assets.join("objects"),
    )
    .ok()?;
    Some(Resources::new(vec![pack]))
}

fn sky_image(i: &Image) -> SkyImage {
    SkyImage {
        width: i.width,
        height: i.height,
        rgba: i.to_rgba8(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_assets::image::argb;
    use ruthenium_auth::AuthError;
    use ruthenium_auth::http::Reply;
    use ruthenium_protocol::packets::Property;
    use ruthenium_protocol::profile::{ResolvableProfile, SkinPatch};
    use serde_json::{Value, json};

    /// The services, as far as these tests ask them; none of it is on the network.
    struct Services {
        keys: bool,
        profile: Option<Value>,
    }

    impl Http for Services {
        fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<Reply, AuthError> {
            unreachable!()
        }
        fn post_json(&self, _: &str, _: &Value, _: Option<&str>) -> Result<Reply, AuthError> {
            unreachable!()
        }
        fn get(&self, url: &str, _: Option<&str>) -> Result<Reply, AuthError> {
            let lines: Vec<&str> = KEY.lines().collect();
            let answer = if url.ends_with("/publickeys") {
                self.keys
                    .then(|| json!({"profilePropertyKeys": [{"publicKey": lines[0]}]}))
            } else {
                self.profile.clone()
            };
            Ok(Reply {
                status: if answer.is_some() { 200 } else { 404 },
                body: answer.unwrap_or_default().to_string(),
            })
        }
    }

    const KEY: &str = include_str!("../../ruthenium-auth/tests/property_key.txt");

    struct Dir(PathBuf);

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Nothing but what is in its cache: a skin called `ab` and a cape called `cd`.
    fn shared(name: &str) -> (Dir, Shared) {
        let dir =
            std::env::temp_dir().join(format!("ruthenium-skins-{name}-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("skins")).unwrap();
        let picture = |w, h| {
            let mut i = Image::new(w, h);
            i.set(8, 8, argb(255, 10, 20, 30));
            i.encode_png()
        };
        std::fs::write(dir.join("skins/ab.png"), picture(64, 64)).unwrap();
        std::fs::write(dir.join("skins/cd.png"), picture(64, 32)).unwrap();
        let s = Shared {
            queue: Mutex::default(),
            changed: Condvar::new(),
            game: None,
            cache: Some(dir.clone()),
            resources: OnceLock::new(),
            keys: Mutex::default(),
        };
        (Dir(dir), s)
    }

    fn value() -> String {
        use base64::Engine;
        let url = |id: &str| format!("http://textures.minecraft.net/texture/{id}");
        base64::engine::general_purpose::STANDARD.encode(
            json!({"textures": {"SKIN": {"url": url("ab"), "metadata": {"model": "slim"}},
                "CAPE": {"url": url("cd")}}})
            .to_string(),
        )
    }

    fn property(value: String, signature: Option<&str>) -> Property {
        Property {
            name: "textures".into(),
            value,
            signature: signature.map(str::to_owned),
        }
    }

    #[test]
    fn a_head_needs_no_signature() {
        let (_dir, s) = shared("head");
        let none = Services {
            keys: false,
            profile: None,
        };
        let p = ResolvableProfile {
            properties: vec![property(value(), None)],
            ..ResolvableProfile::default()
        };
        let found = load(&s, &none, &SkinSource::Profile(p));
        assert!(found.whole);
        let (body, cape) = (found.skin.body.unwrap(), found.skin.cape.unwrap());
        assert_eq!(
            (body.width, body.height, found.skin.slim),
            (64, 64, Some(true))
        );
        assert_eq!((cape.width, cape.height), (64, 32));
        assert_eq!(body.rgba[(8 * 64 + 8) * 4..][..4], [10, 20, 30, 255]);
        // Opaque where it covers the body, clear in the hat.
        assert_eq!((body.rgba[3], body.rgba[40 * 4 + 3]), (255, 0));
    }

    #[test]
    fn a_players_skin_needs_one() {
        let (_dir, s) = shared("player");
        let lines: Vec<&str> = KEY.lines().collect();
        let source = |signature| SkinSource::Player {
            id: 3,
            textures: Some(property(value(), signature)),
        };
        let services = Services {
            keys: true,
            profile: None,
        };
        let unsigned = load(&s, &services, &source(None));
        assert!(unsigned.skin.body.is_none() && unsigned.whole);
        // Signed, but something else.
        let wrong = load(&s, &services, &source(Some(lines[2])));
        assert!(wrong.skin.body.is_none() && wrong.whole);
        // Signed: there is nothing in this one.
        let signed = SkinSource::Player {
            id: 3,
            textures: Some(property(lines[1].into(), Some(lines[2]))),
        };
        assert!(load(&s, &services, &signed).whole);
        // Without the keys it cannot be told: it is tried again.
        let (_dir, s) = shared("keys");
        let none = Services {
            keys: false,
            profile: None,
        };
        assert!(!load(&s, &none, &source(Some(lines[2]))).whole);
    }

    #[test]
    fn a_name_is_looked_up() {
        let (_dir, s) = shared("name");
        let services = Services {
            keys: false,
            profile: Some(
                json!({"id": "0b", "properties": [{"name": "textures", "value": value()}]}),
            ),
        };
        let named = |name: &str, patch| {
            SkinSource::Profile(ResolvableProfile {
                name: Some(name.into()),
                patch,
                ..ResolvableProfile::default()
            })
        };
        let found = load(&s, &services, &named("Kai", SkinPatch::default()));
        assert_eq!(found.skin.id, Some(11));
        assert!(found.skin.body.is_some());
        let wide = SkinPatch {
            slim: Some(false),
            ..SkinPatch::default()
        };
        let found = load(&s, &services, &named("Kai", wide));
        assert_eq!(found.skin.slim, Some(false));
        let none = Services {
            keys: false,
            profile: None,
        };
        let found = load(&s, &none, &named("Kai", SkinPatch::default()));
        assert!(found.skin.id.is_none() && found.whole);
    }

    #[test]
    fn large_pictures_are_not_decoded() {
        let png = Image::new(65, 4).encode_png();
        assert!(decode(&png, 64).is_none() && decode(&png, 65).is_some());
        assert!(decode(&png[..20], 65).is_none());
    }

    #[test]
    fn what_is_asked_for_is_answered() {
        let (dir, _) = shared("loader");
        let loader = SkinLoader::new(None, Some(dir.0.clone()));
        let source = Arc::new(SkinSource::Player {
            id: 1,
            textures: None,
        });
        loader.request(source.clone());
        let started = Instant::now();
        let answer = loop {
            if let Some(a) = loader.take() {
                break a;
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(answer.0, source);
        assert!(answer.1.body.is_none());
        assert!(loader.take().is_none());
    }
}
