//! Block states, from `reports/blocks.json`.
//!
//! Each block's states are numbered consecutively, in block-id order. Within a
//! block, the state is a mixed-radix number over its properties sorted by
//! name, the last property varying fastest; each digit is the index of the
//! value in the property's value list. The report lists properties in
//! declaration order, which can differ (chests: `type`, `facing`, ...), so
//! every state in the report is checked against this numbering.

use std::fmt::Write as _;

use serde_json::Value;

use super::Generated;
use super::{registries, rust};

/// `BlockState` holds a `u16`.
const MAX_STATES: u32 = 1 << 16;

struct Block {
    name: String,
    first_state: u32,
    state_count: u32,
    default_state: u32,
    /// Indexes into the deduplicated property list.
    properties: Vec<usize>,
}

/// A block's properties in state order: (name, values).
type Properties = Vec<(String, Vec<String>)>;

struct Property {
    ident: String,
    name: String,
    values: Vec<String>,
}

pub fn generate(report: &Value, registries_report: &Value) -> Result<Generated, String> {
    let report = report.as_object().ok_or("blocks.json: not an object")?;
    let order = registries::entries(registries_report, "minecraft:block")?;
    if order.len() != report.len() {
        return Err(format!(
            "blocks.json has {} blocks, the block registry {}",
            report.len(),
            order.len()
        ));
    }

    let mut properties: Vec<Property> = Vec::new();
    let mut blocks = Vec::with_capacity(order.len());
    let mut next = 0u32;
    for (name, _) in &order {
        let b = report
            .get(name)
            .ok_or_else(|| format!("blocks.json: no {name}"))?;
        let block = check_block(name, b, next)?;
        let props = block
            .1
            .into_iter()
            .map(|(pname, values)| intern(&mut properties, pname, values))
            .collect();
        next = next
            .checked_add(block.0.state_count)
            .ok_or("too many block states")?;
        blocks.push(Block {
            properties: props,
            ..block.0
        });
    }
    if next > MAX_STATES {
        return Err(format!("{next} block states do not fit BlockState's u16"));
    }

    let mut o =
        String::from("use super::registries::Block;\nuse crate::block::{BlockInfo, Property};\n\n");
    let _ = writeln!(
        o,
        "/// Number of block states: the size of the global block-state palette.\npub const STATE_COUNT: usize = {next};\n"
    );
    for p in &properties {
        let values: Vec<String> = p.values.iter().map(|v| format!("{v:?}")).collect();
        let _ = writeln!(
            o,
            "static {}: Property = Property {{ name: {:?}, values: &[{}] }};",
            p.ident,
            p.name,
            values.join(", ")
        );
    }
    let _ = writeln!(
        o,
        "\n/// By block id.\nstatic BLOCKS: [BlockInfo; {}] = [",
        blocks.len()
    );
    for b in &blocks {
        let props: Vec<String> = b
            .properties
            .iter()
            .map(|&i| format!("&{}", properties[i].ident))
            .collect();
        let _ = writeln!(
            o,
            "    // {}\n    BlockInfo {{ first_state: {}, state_count: {}, default_state: {}, properties: &[{}] }},",
            b.name,
            b.first_state,
            b.state_count,
            b.default_state,
            props.join(", ")
        );
    }
    o.push_str("];\n");
    o.push_str(TEMPLATE);
    Ok(Generated {
        doc: "Block states: per block, its state range, default state and properties.".into(),
        body: o,
    })
}

/// Checks one block's states against the numbering described in the module
/// docs. Returns the block (without interned properties) and its properties
/// in state order.
fn check_block(name: &str, b: &Value, first_state: u32) -> Result<(Block, Properties), String> {
    let what = format!("blocks.json: {name}");
    let mut props: Properties = match b.get("properties") {
        None => Vec::new(),
        Some(p) => p
            .as_object()
            .ok_or_else(|| format!("{what}: properties is not an object"))?
            .iter()
            .map(|(pname, values)| {
                let values = values
                    .as_array()
                    .filter(|a| !a.is_empty())
                    .ok_or_else(|| format!("{what}: {pname} has no values"))?
                    .iter()
                    .map(|v| rust::str_of(v, &what).map(str::to_owned))
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((pname.clone(), values))
            })
            .collect::<Result<_, String>>()?,
    };
    props.sort_by(|a, b| a.0.cmp(&b.0));

    let states = b["states"]
        .as_array()
        .ok_or_else(|| format!("{what}: no states"))?;
    let count = props
        .iter()
        .try_fold(1u32, |n, (_, v)| n.checked_mul(v.len() as u32))
        .ok_or_else(|| format!("{what}: too many states"))?;
    if states.len() != count as usize {
        return Err(format!(
            "{what}: {} states, but its properties make {count}",
            states.len()
        ));
    }
    let mut default_state = None;
    for (i, state) in states.iter().enumerate() {
        let id = rust::u32_of(&state["id"], &what)?;
        let expected_id = first_state + i as u32;
        if id != expected_id {
            return Err(format!(
                "{what}: state #{i} has id {id}, expected {expected_id}"
            ));
        }
        let actual = state.get("properties").and_then(Value::as_object);
        if actual.map_or(0, |a| a.len()) != props.len() {
            return Err(format!("{what}: state {id} has the wrong properties"));
        }
        let mut rest = i;
        for (pname, values) in props.iter().rev() {
            let expected = &values[rest % values.len()];
            rest /= values.len();
            let got = actual.and_then(|a| a.get(pname)).and_then(Value::as_str);
            if got != Some(expected.as_str()) {
                return Err(format!(
                    "{what}: state {id} has {pname}={got:?}, but the numbering gives {expected:?}"
                ));
            }
        }
        if state.get("default").and_then(Value::as_bool) == Some(true) {
            if default_state.is_some() {
                return Err(format!("{what}: more than one default state"));
            }
            default_state = Some(id);
        }
    }
    let default_state = default_state.ok_or_else(|| format!("{what}: no default state"))?;
    Ok((
        Block {
            name: name.to_owned(),
            first_state,
            state_count: count,
            default_state,
            properties: Vec::new(),
        },
        props,
    ))
}

/// Returns the index of the property with this name and value list, adding
/// it if new. Properties that share a name but not their values get numbered
/// identifiers (`FACING`, `FACING_2`, ...).
fn intern(properties: &mut Vec<Property>, name: String, values: Vec<String>) -> usize {
    if let Some(i) = properties
        .iter()
        .position(|p| p.name == name && p.values == values)
    {
        return i;
    }
    let same_name = properties.iter().filter(|p| p.name == name).count();
    let mut ident = rust::screaming_snake(&name);
    if same_name > 0 {
        ident = format!("{ident}_{}", same_name + 1);
    }
    properties.push(Property {
        ident,
        name,
        values,
    });
    properties.len() - 1
}

/// The typed API over the tables above. Identical for every version.
const TEMPLATE: &str = r#"
static STATE_BLOCK: [u16; STATE_COUNT] = crate::block::state_to_block(&BLOCKS);

/// A block state: its id in the global palette (`0..STATE_COUNT`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockState(u16);

impl BlockState {
    /// The state with this palette id, if there is one.
    pub const fn from_id(id: u32) -> Option<Self> {
        if (id as usize) < STATE_COUNT {
            Some(Self(id as u16))
        } else {
            None
        }
    }

    /// The palette id.
    pub const fn id(self) -> u32 {
        self.0 as u32
    }

    /// The block this is a state of.
    pub fn block(self) -> Block {
        Block::ALL[STATE_BLOCK[self.0 as usize] as usize]
    }

    /// Whether this is its block's default state.
    pub fn is_default(self) -> bool {
        self.info().default_state == self.id()
    }

    /// The value of a property, e.g. `value("facing") == Some("north")`.
    pub fn value(self, property: &str) -> Option<&'static str> {
        self.info().value(self.id(), property)
    }

    /// This state with one property changed, if the block has that property
    /// and value.
    pub fn with(self, property: &str, value: &str) -> Option<Self> {
        self.info()
            .with_value(self.id(), property, value)
            .map(|id| Self(id as u16))
    }

    /// Every `(property, value)` of this state, in property-name order.
    pub fn values(self) -> impl Iterator<Item = (&'static str, &'static str)> {
        self.info().values(self.id())
    }

    fn info(self) -> &'static BlockInfo {
        self.block().info()
    }
}

/// Formats like vanilla: `minecraft:oak_stairs[facing=north,half=bottom,...]`.
impl core::fmt::Display for BlockState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.block().name())?;
        let mut sep = '[';
        for (property, value) in self.values() {
            write!(f, "{sep}{property}={value}")?;
            sep = ',';
        }
        if sep == ',' {
            f.write_str("]")?;
        }
        Ok(())
    }
}

impl Block {
    /// The block's state range, default state and properties.
    pub fn info(self) -> &'static BlockInfo {
        &BLOCKS[self as usize]
    }

    /// The state the block is placed in when nothing says otherwise.
    pub fn default_state(self) -> BlockState {
        BlockState(self.info().default_state as u16)
    }

    /// All of the block's states, in id order.
    pub fn states(self) -> impl Iterator<Item = BlockState> {
        self.info().states().map(|id| BlockState(id as u16))
    }
}
"#;

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::check_block;

    #[test]
    fn numbers_states_by_sorted_property_names() {
        // Declared as (type, facing): numbered as (facing, type), type fastest.
        let block = json!({
            "properties": { "type": ["normal", "sticky"], "facing": ["north", "up"] },
            "states": [
                { "id": 10, "properties": { "type": "normal", "facing": "north" }, "default": true },
                { "id": 11, "properties": { "type": "sticky", "facing": "north" } },
                { "id": 12, "properties": { "type": "normal", "facing": "up" } },
                { "id": 13, "properties": { "type": "sticky", "facing": "up" } }
            ]
        });
        let (b, props) = check_block("minecraft:x", &block, 10).unwrap();
        assert_eq!((b.state_count, b.default_state), (4, 10));
        assert_eq!(props[0].0, "facing");
        assert!(check_block("minecraft:x", &block, 9).is_err());
    }

    #[test]
    fn rejects_other_numberings() {
        let block = json!({
            "properties": { "a": ["0", "1"], "b": ["0", "1"] },
            "states": [
                { "id": 0, "properties": { "a": "0", "b": "0" }, "default": true },
                { "id": 1, "properties": { "a": "1", "b": "0" } },
                { "id": 2, "properties": { "a": "0", "b": "1" } },
                { "id": 3, "properties": { "a": "1", "b": "1" } }
            ]
        });
        assert!(check_block("minecraft:x", &block, 0).is_err());
    }
}
