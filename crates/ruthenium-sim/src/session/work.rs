//! The workstations' side of the session: what their menus read of the session (the
//! registries' tags, the stonecutter's recipes, the banner patterns, the language), the
//! packets their screens send, and the merchant's offers.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::packets::configuration::RegistryTags;
use ruthenium_protocol::recipe::StonecutterEntry;
use ruthenium_protocol::trade::MerchantOffer;

use super::Game;
use crate::menu::work::{PlayerFacts, WorkData};

/// What the session keeps for the workstations.
#[derive(Debug, Default)]
pub(super) struct WorkSession {
    /// Every registry's tags as the server last sent them (`update_tags` replaces a
    /// registry's whole set).
    tags: HashMap<String, HashMap<String, Vec<i32>>>,
    pub(super) stonecutter: Vec<StonecutterEntry>,
    pub(super) language: Arc<HashMap<String, String>>,
}

impl WorkSession {
    pub(super) fn set_tags(&mut self, tags: &[RegistryTags]) {
        for t in tags {
            self.tags
                .insert(t.registry.clone(), t.tags.iter().cloned().collect());
        }
    }
}

impl Game {
    /// What a workstation's menu reads, as of now.
    fn work_data(&self) -> Arc<WorkData> {
        let banner_patterns = self
            .configured
            .registries
            .iter()
            .find(|(r, _)| r == "minecraft:banner_pattern")
            .map(|(_, e)| WorkData::banner_patterns_from(e))
            .unwrap_or_default();
        Arc::new(WorkData {
            items: self
                .physics
                .as_ref()
                .map(|p| p.items.clone())
                .unwrap_or_default(),
            tags: self.containers.work.tags.clone(),
            stonecutter: self.containers.work.stonecutter.clone(),
            banner_patterns,
            language: self.containers.work.language.clone(),
        })
    }

    /// A workstation's menu just opened: it gets what it reads.
    pub(super) fn work_opened(&mut self) {
        let data = self.work_data();
        if let Some(w) = &mut self.world
            && let Some(state) = w.player.menus.open.as_mut().and_then(|m| m.work_mut())
        {
            state.data = data;
        }
        self.work_sync();
    }

    /// The player's levels and game mode, as the open workstation reads them.
    pub(super) fn work_sync(&mut self) {
        if let Some(w) = &mut self.world {
            let facts = PlayerFacts {
                creative: w.player.creative,
                xp_level: w.player.experience_level,
            };
            if let Some(state) = w.player.menus.open.as_mut().and_then(|m| m.work_mut()) {
                state.player = facts;
            }
        }
    }

    /// Sends what the open workstation queued (an anvil's rename after a slot packet).
    pub(super) fn work_outbox(&mut self) {
        let Some(w) = &mut self.world else {
            return;
        };
        let packets = w.player.menus.container_mut().take_outbox();
        for p in packets {
            log_sent(&p);
            self.connection.send(p);
        }
    }

    /// `handleMerchantOffers`: for the open merchant menu only.
    pub(super) fn merchant_offers(
        &mut self,
        container_id: i32,
        offers: &[MerchantOffer],
        level: i32,
        xp: i32,
        show_progress: bool,
        can_restock: bool,
    ) {
        let data = self.work_data();
        let Some(w) = &mut self.world else {
            return;
        };
        let menu = w.player.menus.container_mut();
        if menu.id != container_id || menu.station() != Some(crate::menu::work::Station::Merchant) {
            return;
        }
        if crate::menu::log_enabled() {
            eprintln!(
                "[menu] got merchant_offers container {container_id}: {} offers, level {level}, xp {xp}",
                offers.len()
            );
        }
        crate::menu::work::merchant::set_offers(
            menu,
            &data,
            offers,
            level,
            xp,
            show_progress,
            can_restock,
        );
    }
}

/// `RUTHENIUM_MENU_LOG`: a workstation's own packets as they go.
pub(super) fn log_sent(p: &[u8]) {
    use ruthenium_data::v26_3::packets::play::Serverbound as S;
    if !crate::menu::log_enabled() {
        return;
    }
    let Some(&id) = p.first() else {
        return;
    };
    let mut r = ruthenium_protocol::wire::Reader::new(&p[1..]);
    let what = match S::from_id(u32::from(id)) {
        Some(S::RenameItem) => format!("rename_item {:?}", r.string(32767).unwrap_or("?")),
        Some(S::ContainerButtonClick) => format!(
            "container_button_click container {} button {}",
            r.varint().unwrap_or(-1),
            r.varint().unwrap_or(-1)
        ),
        Some(S::SelectTrade) => format!("select_trade {}", r.varint().unwrap_or(-1)),
        Some(S::ContainerSlotStateChanged) => format!(
            "container_slot_state_changed slot {} container {} {}",
            r.varint().unwrap_or(-1),
            r.varint().unwrap_or(-1),
            r.bool().unwrap_or(false)
        ),
        _ => return,
    };
    eprintln!("[menu] sent {what}");
}
