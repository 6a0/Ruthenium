//! The beacon's, the enchanting table's and the lectern's screens in a session: what
//! their buttons and keys send (`super::super::stations`).

use super::*;
use crate::session::stations::ButtonKind;
use ruthenium_data::v26_3::packets::play as pl;
use ruthenium_data::v26_3::registries::{Item, Menu as MenuType};

fn open(r: &mut Rig, id: i32, name: &str) {
    r.packet(play::Clientbound::OpenScreen {
        container_id: id,
        menu_type: MenuType::from_name(name).unwrap().id() as i32,
        title: Tag::String("A screen".into()),
    });
    r.frame_with(
        1,
        FrameInput {
            pointer: Some(Pointer {
                x: 0.0,
                y: 0.0,
                width: 400,
                height: 300,
            }),
            ..FrameInput::default()
        },
    );
    assert!(r.game.container_screen_open());
}

/// The menus' packets sent.
fn sent(r: &mut Rig) -> Vec<Vec<u8>> {
    let menus = [
        pl::Serverbound::ContainerClick,
        pl::Serverbound::ContainerClose,
        pl::Serverbound::ContainerButtonClick,
        pl::Serverbound::SetBeacon,
    ]
    .map(|p| p.id() as u8);
    r.remote
        .sent()
        .into_iter()
        .filter_map(|s| match s {
            Sent::Packet(p) if menus.contains(&p[0]) => Some(p),
            _ => None,
        })
        .collect()
}

fn press(r: &mut Rig, x: f64, y: f64, button: u8) {
    r.frame_with(
        1,
        FrameInput {
            screen_inputs: vec![
                ScreenInput::Press {
                    button,
                    at: At::Gui(x, y),
                    shift: false,
                    binds: crate::menu::screen::Binds::default(),
                    when: std::time::Instant::now(),
                },
                ScreenInput::Release {
                    button,
                    at: At::Gui(x, y),
                    shift: false,
                    binds: crate::menu::screen::Binds::default(),
                },
            ],
            ..FrameInput::default()
        },
    );
}

fn key(r: &mut Rig, binds: crate::menu::screen::Binds) {
    r.frame_with(
        1,
        FrameInput {
            screen_inputs: vec![ScreenInput::Key {
                binds,
                ctrl: false,
                at: At::Outside,
            }],
            ..FrameInput::default()
        },
    );
}

/// The middle of the first button the view has of `kind`.
fn button(r: &Rig, f: impl Fn(&ButtonKind) -> bool) -> crate::session::Button {
    *r.game
        .station_view()
        .unwrap()
        .buttons
        .iter()
        .find(|b| f(&b.kind))
        .expect("the button")
}

fn middle(b: &crate::session::Button) -> (f64, f64) {
    (f64::from(b.x + b.width / 2), f64::from(b.y + b.height / 2))
}

fn stack(name: &str, count: i32) -> crate::item::Stack {
    let mut s =
        crate::item::Items::default().new_stack(Item::from_name(name).unwrap().id() as i32, 1);
    s.count = count;
    s
}

fn own(r: &mut Rig) -> &mut Vec<crate::item::Stack> {
    &mut r
        .game
        .world
        .as_mut()
        .unwrap()
        .player
        .menus
        .open
        .as_mut()
        .unwrap()
        .own
}

#[test]
fn a_beacons_buttons_choose_and_send_its_effects() {
    let mut r = start(None);
    open(&mut r, 2, "minecraft:beacon");
    let speed = |k: &ButtonKind| {
        matches!(
            k,
            ButtonKind::BeaconEffect {
                effect: 0,
                primary: true,
                ..
            }
        )
    };
    // Until a tick has looked at the levels, no effect can be chosen.
    assert!(!button(&r, speed).active);
    r.packet(play::Clientbound::ContainerSetData {
        container_id: 2,
        id: 0,
        value: 2,
    });
    r.frame(100);
    let b = button(&r, speed);
    assert!(b.active && !b.selected);
    // A third-tier effect needs three levels.
    let strength = |k: &ButtonKind| matches!(k, ButtonKind::BeaconEffect { tier: 2, .. });
    assert!(!button(&r, strength).active);
    let (x, y) = middle(&b);
    press(&mut r, x, y, 1);
    let b = button(&r, speed);
    assert!(b.selected && b.focused);
    let upgrade = |k: &ButtonKind| matches!(k, ButtonKind::BeaconEffect { upgrade: true, .. });
    assert!(button(&r, upgrade).visible);
    // Without a payment Done is off: the press goes to the window, and nothing is sent.
    let done = button(&r, |k| *k == ButtonKind::BeaconConfirm);
    assert!(!done.active);
    let (x, y) = middle(&done);
    press(&mut r, x, y, 1);
    assert!(r.game.container_screen_open());
    assert_eq!(sent(&mut r), Vec::<Vec<u8>>::new());
    own(&mut r)[0] = stack("minecraft:emerald", 1);
    r.frame(100);
    assert!(button(&r, |k| *k == ButtonKind::BeaconConfirm).active);
    // The right button presses nothing.
    press(&mut r, x, y, 3);
    assert!(r.game.container_screen_open());
    press(&mut r, x, y, 1);
    assert!(!r.game.container_screen_open());
    assert_eq!(
        sent(&mut r),
        [play::set_beacon(Some(0), None), play::container_close(2)]
    );
    // The server's data choose them too: haste (2) and regeneration (9), each + 1.
    open(&mut r, 3, "minecraft:beacon");
    for (id, value) in [(0, 4), (1, 3), (2, 10)] {
        r.packet(play::Clientbound::ContainerSetData {
            container_id: 3,
            id,
            value,
        });
    }
    r.frame(100);
    let haste = button(&r, |k| {
        matches!(
            k,
            ButtonKind::BeaconEffect {
                effect: 2,
                primary: true,
                ..
            }
        )
    });
    assert!(haste.selected);
    let regen = button(&r, |k| {
        matches!(
            k,
            ButtonKind::BeaconEffect {
                effect: 9,
                upgrade: false,
                ..
            }
        )
    });
    assert!(regen.selected && regen.active);
    // Another primary takes the secondary away, unless it is that one.
    let speed_button = button(&r, speed);
    let (x, y) = middle(&speed_button);
    press(&mut r, x, y, 1);
    assert!(
        !button(&r, |k| matches!(
            k,
            ButtonKind::BeaconEffect {
                effect: 9,
                upgrade: false,
                ..
            }
        ))
        .selected
    );
    let upgrade_button = button(&r, upgrade);
    let (x, y) = middle(&upgrade_button);
    press(&mut r, x, y, 1);
    assert!(button(&r, upgrade).selected);
    let (x, y) = middle(&speed_button);
    press(&mut r, x, y, 1);
    assert!(button(&r, upgrade).selected);
    let cancel = button(&r, |k| *k == ButtonKind::BeaconCancel);
    let (x, y) = middle(&cancel);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), [play::container_close(3)]);
}

#[test]
fn an_enchanting_tables_offers_are_sent_when_they_may_be_taken() {
    let mut r = start(None);
    open(&mut r, 4, "minecraft:enchantment");
    r.packet(play::Clientbound::SetExperience {
        progress: 0.0,
        level: 10,
        total: 160,
    });
    for (id, value) in [(0, 1), (1, 5), (2, 30)] {
        r.packet(play::Clientbound::ContainerSetData {
            container_id: 4,
            id,
            value,
        });
    }
    r.frame(1);
    own(&mut r)[0] = stack("minecraft:diamond_sword", 1);
    own(&mut r)[1] = stack("minecraft:lapis_lazuli", 2);
    // The window's corner on a 400 by 300 GUI, and each offer's middle.
    let (left, top) = (f64::from((400 - 176) / 2), f64::from((300 - 166) / 2));
    let offer = |i: i32| (left + 60.0 + 54.0, top + 14.0 + 19.0 * f64::from(i) + 9.0);
    let (x, y) = offer(0);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), [play::container_button_click(4, 0)]);
    // Any button; the third wants 30 levels and three lapis.
    let (x, y) = offer(1);
    press(&mut r, x, y, 3);
    assert_eq!(sent(&mut r), [play::container_button_click(4, 1)]);
    let (x, y) = offer(2);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), Vec::<Vec<u8>>::new());
    // Levels enough, lapis not.
    r.game.world.as_mut().unwrap().player.experience_level = 30;
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), Vec::<Vec<u8>>::new());
    r.game.world.as_mut().unwrap().player.experience_level = 10;
    // An item taken away takes the offers along (`slotsChanged`).
    own(&mut r)[1] = stack("minecraft:lapis_lazuli", 3);
    r.game.world.as_mut().unwrap().player.experience_level = 30;
    let (x, y) = offer(2);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), [play::container_button_click(4, 2)]);
}

/// A written book's component: its title, author, generation, these pages (string
/// tags) and resolved.
fn written_book(pages: &[&str]) -> crate::item::Stack {
    use ruthenium_data::v26_3::registries::DataComponentType as K;
    use ruthenium_protocol::wire::{write_string, write_varint};
    let mut d = Vec::new();
    write_string(&mut d, "Title");
    d.push(0);
    write_string(&mut d, "Author");
    write_varint(&mut d, 0);
    write_varint(&mut d, pages.len() as i32);
    for p in pages {
        d.push(8);
        d.extend_from_slice(&(p.len() as u16).to_be_bytes());
        d.extend_from_slice(p.as_bytes());
        d.push(0);
    }
    d.push(1);
    crate::item::Items::default().stack(&ruthenium_protocol::item::ItemStack {
        item: Item::WrittenBook.id() as i32,
        count: 1,
        patch: ruthenium_protocol::item::ComponentPatch {
            added: vec![(
                K::WrittenBookContent.id() as i32,
                ruthenium_protocol::item::Component::Other {
                    kind: K::WrittenBookContent.id() as i32,
                    data: d,
                },
            )],
            ..Default::default()
        },
    })
}

#[test]
fn a_lecterns_book_turns_its_pages_on_the_server() {
    use crate::menu::screen::Binds;
    let mut r = start(None);
    r.game.world.as_mut().unwrap().player.abilities.may_build = true;
    open(&mut r, 6, "minecraft:lectern");
    own(&mut r)[0] = written_book(&["one", "two", "three"]);
    r.packet(play::Clientbound::ContainerSetData {
        container_id: 6,
        id: 0,
        value: 1,
    });
    r.frame(1);
    let v = r.game.station_view().unwrap();
    assert_eq!((v.pages.len(), v.page), (3, 1));
    assert_eq!(*v.pages[2], Tag::String("three".into()));
    let back = button(&r, |k| *k == ButtonKind::PageBack);
    assert!(back.visible && button(&r, |k| *k == ButtonKind::PageForward).visible);
    let (x, y) = middle(&back);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), [play::container_button_click(6, 1)]);
    // The page shown is the server's: at the last, there is no next page.
    r.packet(play::Clientbound::ContainerSetData {
        container_id: 6,
        id: 0,
        value: 7,
    });
    r.frame(1);
    assert_eq!(r.game.station_view().unwrap().page, 2);
    assert!(!button(&r, |k| *k == ButtonKind::PageForward).visible);
    key(
        &mut r,
        Binds {
            page_up: true,
            ..Binds::default()
        },
    );
    assert_eq!(sent(&mut r), [play::container_button_click(6, 1)]);
    // Page Down presses it all the same; the inventory key does nothing here.
    key(
        &mut r,
        Binds {
            page_down: true,
            ..Binds::default()
        },
    );
    key(
        &mut r,
        Binds {
            inventory: true,
            ..Binds::default()
        },
    );
    assert!(r.game.container_screen_open());
    assert_eq!(sent(&mut r), [play::container_button_click(6, 2)]);
    let take = button(&r, |k| *k == ButtonKind::TakeBook);
    let (x, y) = middle(&take);
    press(&mut r, x, y, 1);
    assert_eq!(sent(&mut r), [play::container_button_click(6, 3)]);
    key(
        &mut r,
        Binds {
            escape: true,
            ..Binds::default()
        },
    );
    assert!(!r.game.container_screen_open());
    assert_eq!(sent(&mut r), [play::container_close(6)]);
    // A player that may not build has Done alone, as wide as two.
    r.game.world.as_mut().unwrap().player.abilities.may_build = false;
    open(&mut r, 7, "minecraft:lectern");
    let v = r.game.station_view().unwrap();
    assert!(!v.buttons.iter().any(|b| b.kind == ButtonKind::TakeBook));
    assert_eq!(button(&r, |k| *k == ButtonKind::Done).width, 200);
}

#[test]
fn a_mounts_inventory_opens_with_its_slots() {
    use ruthenium_data::v26_3::registries::EntityType;
    // (With the prepared game: entities need its types.)
    let Some(physics) = physics() else {
        return;
    };
    let mut r = rig(Some(physics));
    r.packet(login());
    r.frame(50);
    for (entity_id, name) in [
        (900, "minecraft:llama"),
        (901, "minecraft:donkey"),
        (902, "minecraft:pig"),
    ] {
        r.packet(play::Clientbound::AddEntity(Box::new(play::AddEntity {
            entity_id,
            uuid: entity_id as u128,
            entity_type: EntityType::from_name(name).unwrap().id() as i32,
            position: [0.0, 10.0, 0.0],
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: 0,
        })));
    }
    r.frame(1);
    r.packet(play::Clientbound::MountScreenOpen {
        container_id: 8,
        inventory_columns: 3,
        entity_id: 900,
    });
    r.frame(1);
    let menu = r.game.world.as_ref().unwrap().player.menus.container();
    let crate::menu::Kind::Mount(m) = menu.kind else {
        panic!("{:?}", menu.kind);
    };
    assert_eq!((menu.id, menu.slots.len(), m.columns), (8, 2 + 9 + 36, 3));
    // A llama wears a carpet, and has no saddle.
    assert!(!m.saddle_shown && m.body_shown && m.llama);
    // An untamed donkey shows neither.
    r.packet(play::Clientbound::MountScreenOpen {
        container_id: 9,
        inventory_columns: 0,
        entity_id: 901,
    });
    r.frame(1);
    let menu = r.game.world.as_ref().unwrap().player.menus.container();
    let crate::menu::Kind::Mount(m) = menu.kind else {
        panic!();
    };
    assert_eq!(menu.slots.len(), 2 + 36);
    assert!(!m.saddle && !m.saddle_shown && !m.body_shown);
    // A horse wears a saddle and armour by the server's entity type tags.
    let horse = EntityType::from_name("minecraft:horse").unwrap().id() as i32;
    r.packet(play::Clientbound::UpdateTags(vec![
        ruthenium_protocol::packets::configuration::RegistryTags {
            registry: "minecraft:entity_type".into(),
            tags: vec![
                ("minecraft:can_equip_saddle".into(), vec![horse]),
                ("minecraft:can_wear_horse_armor".into(), vec![horse]),
            ],
        },
    ]));
    r.packet(play::Clientbound::AddEntity(Box::new(play::AddEntity {
        entity_id: 903,
        uuid: 903,
        entity_type: horse,
        position: [0.0, 10.0, 0.0],
        movement: [0.0; 3],
        x_rot: 0,
        y_rot: 0,
        y_head_rot: 0,
        data: 0,
    })));
    r.frame(1);
    r.packet(play::Clientbound::MountScreenOpen {
        container_id: 11,
        inventory_columns: 0,
        entity_id: 903,
    });
    r.frame(1);
    let menu = r.game.world.as_ref().unwrap().player.menus.container();
    let crate::menu::Kind::Mount(m) = menu.kind else {
        panic!();
    };
    assert!(m.saddle_shown && m.body_shown, "{m:?}");
    // A pig has no inventory screen: nothing opens.
    r.packet(play::Clientbound::MountScreenOpen {
        container_id: 10,
        inventory_columns: 0,
        entity_id: 902,
    });
    r.frame(1);
    assert_eq!(
        r.game.world.as_ref().unwrap().player.menus.container().id,
        11
    );
}
