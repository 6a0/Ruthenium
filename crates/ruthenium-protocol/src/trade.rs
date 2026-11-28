//! A merchant's offers as the server sends them (`MerchantOffer.STREAM_CODEC`,
//! `ItemCost.STREAM_CODEC`).

use crate::item::{self, Component, ItemStack};
use crate::wire::{Reader, WireError};

/// `ItemCost`: an item, how many, and the components the stack must have exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemCost {
    pub item: i32,
    pub count: i32,
    /// `DataComponentExactPredicate`: (type id, value).
    pub components: Vec<(i32, Component)>,
}

impl ItemCost {
    pub fn read(r: &mut Reader<'_>) -> Result<ItemCost, WireError> {
        Ok(ItemCost {
            item: r.varint()?,
            count: r.varint()?,
            components: item::read_exact_components(r)?,
        })
    }
}

/// `MerchantOffer` as the client gets it.
#[derive(Debug, Clone, PartialEq)]
pub struct MerchantOffer {
    pub cost_a: ItemCost,
    pub result: ItemStack,
    pub cost_b: Option<ItemCost>,
    /// The offer is sold out (`setToOutOfStock`: its uses are set to its maximum).
    pub out_of_stock: bool,
    pub uses: i32,
    pub max_uses: i32,
    pub xp: i32,
    pub special_price_diff: i32,
    pub price_multiplier: f32,
    pub demand: i32,
}

impl MerchantOffer {
    /// `createFromStream`.
    pub fn read(r: &mut Reader<'_>) -> Result<MerchantOffer, WireError> {
        let cost_a = ItemCost::read(r)?;
        let result = ItemStack::read(r)?;
        let cost_b = if r.bool()? {
            Some(ItemCost::read(r)?)
        } else {
            None
        };
        Ok(MerchantOffer {
            cost_a,
            result,
            cost_b,
            out_of_stock: r.bool()?,
            uses: r.i32()?,
            max_uses: r.i32()?,
            xp: r.i32()?,
            special_price_diff: r.i32()?,
            price_multiplier: r.f32()?,
            demand: r.i32()?,
        })
    }
}

/// `MerchantOffers.STREAM_CODEC`: a list of offers.
pub fn read_offers(r: &mut Reader<'_>) -> Result<Vec<MerchantOffer>, WireError> {
    let n = r.count(usize::MAX)?;
    let mut offers = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        offers.push(MerchantOffer::read(r)?);
    }
    Ok(offers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_read_as_vanilla_writes_them() {
        let mut b = vec![1];
        // Cost A: item 5, 12 of them, no components; the result: 3 of item 9.
        b.extend([5, 12, 0, 3, 9, 0, 0]);
        // Cost B: item 6, one, with damage 4 (type 3).
        b.extend([1, 6, 1, 1, 3, 4]);
        b.push(0);
        for v in [2i32, 12, 5, -3] {
            b.extend(v.to_be_bytes());
        }
        b.extend(0.05f32.to_be_bytes());
        b.extend(7i32.to_be_bytes());
        let offers = read_offers(&mut Reader::new(&b)).unwrap();
        let o = &offers[0];
        assert_eq!((o.cost_a.item, o.cost_a.count), (5, 12));
        assert_eq!((o.result.item, o.result.count), (9, 3));
        let b = o.cost_b.as_ref().unwrap();
        assert_eq!(b.components, [(3, Component::Damage(4))]);
        assert!(!o.out_of_stock);
        assert_eq!(
            (o.uses, o.max_uses, o.xp, o.special_price_diff, o.demand),
            (2, 12, 5, -3, 7)
        );
        assert_eq!(o.price_multiplier, 0.05);
    }
}
