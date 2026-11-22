//! `ruthenium assets`: builds the sprite atlases from the imported game
//! files (and any resource packs given), reports them, and can dump them.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ruthenium_assets::atlas::{self, AtlasOptions};
use ruthenium_assets::{Pack, Resources};
use ruthenium_prepare::discover::Env;
use ruthenium_prepare::{import, supported_version};

#[derive(Debug, Default)]
struct Options {
    data_dir: Option<PathBuf>,
    packs: Vec<PathBuf>,
    dump: Option<PathBuf>,
    mip_levels: Option<u32>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--data-dir" => o.data_dir = Some(value()?.into()),
            "--pack" => o.packs.push(value()?.into()),
            "--dump" => o.dump = Some(value()?.into()),
            "--mip-levels" => {
                let v = value()?;
                o.mip_levels = Some(
                    v.parse()
                        .ok()
                        .filter(|n| *n <= 4)
                        .ok_or_else(|| format!("bad --mip-levels {v:?} (0-4)"))?,
                );
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(o)
}

pub fn run(args: &[String]) -> ExitCode {
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}\n\n{}", crate::USAGE);
            return ExitCode::FAILURE;
        }
    };
    let Some(data_dir) = opts
        .data_dir
        .clone()
        .or_else(|| ruthenium_prepare::default_data_dir(&Env::from_process()))
    else {
        eprintln!("cannot work out a data folder; pass --data-dir <folder>");
        return ExitCode::FAILURE;
    };
    let version = supported_version();
    let Some(imported) = import::open(version, &data_dir.join("game")) else {
        eprintln!(
            "No imported Minecraft {} in {}. Run `ruthenium prepare` first.",
            version.id,
            data_dir.display()
        );
        return ExitCode::FAILURE;
    };

    let start = Instant::now();
    let mut packs = Vec::new();
    match Pack::vanilla(
        &imported.client_jar,
        &imported.asset_index(),
        &imported.assets.join("objects"),
    ) {
        Ok(p) => packs.push(p),
        Err(e) => {
            eprintln!("cannot open the game files: {e}");
            return ExitCode::FAILURE;
        }
    }
    for path in &opts.packs {
        match Pack::open(path) {
            Ok(p) => packs.push(p),
            Err(e) => {
                eprintln!("resource pack skipped: {e}");
            }
        }
    }
    let res = Resources::new(packs);
    let opened = start.elapsed();
    let options = AtlasOptions {
        mip_levels: opts
            .mip_levels
            .unwrap_or(AtlasOptions::default().mip_levels),
        ..AtlasOptions::default()
    };
    let atlases = atlas::build_all(&res, &options);
    let built = start.elapsed();

    let mut ok = true;
    for a in &atlases {
        match a {
            Ok(a) => {
                println!(
                    "{:<16} {:>5}x{:<5} mips {}  {:>4} sprites ({} animated){}",
                    a.def.name,
                    a.width,
                    a.height,
                    a.max_level,
                    a.sprites.len(),
                    a.animated.len(),
                    if a.warnings.is_empty() {
                        String::new()
                    } else {
                        format!(", {} warnings", a.warnings.len())
                    }
                );
                for w in &a.warnings {
                    println!("    {w}");
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ok = false;
            }
        }
    }
    println!(
        "Opened packs in {opened:.1?}; built every atlas in {:.1?}",
        built - opened
    );

    if let Some(dir) = &opts.dump {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!("{}: {e}", dir.display());
            return ExitCode::FAILURE;
        }
        for a in atlases.iter().flatten() {
            for (l, img) in a.levels.iter().enumerate() {
                let path = dir.join(format!("{}_{l}.png", a.def.name));
                if let Err(e) = std::fs::write(&path, img.encode_png()) {
                    eprintln!("{}: {e}", path.display());
                    ok = false;
                }
            }
            let path = dir.join(format!("{}.txt", a.def.name));
            if let Err(e) = std::fs::write(&path, a.dump_index()) {
                eprintln!("{}: {e}", path.display());
                ok = false;
            }
        }
        println!("Dumped to {}", dir.display());
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
