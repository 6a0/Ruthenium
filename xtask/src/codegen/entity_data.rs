//! Entity-data serializer ids.
//!
//! A serializer's wire id is the order in which `EntityDataSerializers`
//! registers it, and that order differs from the order the constants are
//! declared in. It is read from the jar's bytecode: the static initializer
//! calls `registerSerializer` once per serializer, each time with a constant
//! of the class just loaded by `getstatic`.

use super::classfile::{self, GETSTATIC, INVOKESTATIC};
use super::{Generated, rust};

pub const CLASS: &str = "net/minecraft/network/syncher/EntityDataSerializers.class";
const REGISTER: &str = "registerSerializer";

pub fn generate(class_bytes: &[u8]) -> Result<Generated, String> {
    let names = registration_order(class_bytes, REGISTER).map_err(|e| format!("{CLASS}: {e}"))?;
    let entries = names
        .iter()
        .zip(0..)
        .map(|(n, id)| (n.to_ascii_lowercase(), id))
        .collect();
    let variants = rust::variants(entries, "entity-data serializers")?;
    let mut body = String::new();
    rust::id_enum(
        &mut body,
        "",
        "Entity-data serializers, by the id `set_entity_data` carries for each value.",
        "EntityDataSerializer",
        &variants,
    );
    Ok(Generated {
        doc: "Entity-data serializer ids (registration order in `EntityDataSerializers`).".into(),
        body,
    })
}

/// The constants passed to `method` by the static initializer, in call order.
/// Fails unless every call to `method` takes a constant of this class
/// loaded just before it, so a change in how registration is written cannot
/// silently drop or reorder ids.
fn registration_order(class_bytes: &[u8], method: &str) -> Result<Vec<String>, String> {
    let class = classfile::parse(class_bytes)?;
    let this = class.name()?;
    let code = class.code("<clinit>").ok_or("no static initializer")?;
    let code = classfile::instructions(code)?;

    let is_register = |i: &classfile::Instruction<'_>| -> Result<bool, String> {
        if i.opcode != INVOKESTATIC {
            return Ok(false);
        }
        let m = class.member(i.pool_index().ok_or("bad invokestatic")?)?;
        Ok(m.class == this && m.name == method)
    };
    let mut calls = 0;
    let mut names = Vec::new();
    for (k, insn) in code.iter().enumerate() {
        if !is_register(insn)? {
            continue;
        }
        calls += 1;
        let Some(prev) = k.checked_sub(1).map(|p| &code[p]) else {
            continue;
        };
        if prev.opcode != GETSTATIC {
            continue;
        }
        let field = class.member(prev.pool_index().ok_or("bad getstatic")?)?;
        if field.class == this {
            names.push(field.name.to_owned());
        }
    }
    if names.is_empty() || names.len() != calls {
        return Err(format!(
            "{calls} call(s) to {method}, {} of them with a constant of this class",
            names.len()
        ));
    }
    let mut sorted = names.clone();
    sorted.sort();
    if let Some(w) = sorted.windows(2).find(|w| w[0] == w[1]) {
        return Err(format!("{} is registered twice", w[0]));
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::registration_order;

    /// A minimal class `T` whose static initializer registers `A`, runs a
    /// tableswitch (variable length), then registers `B`.
    fn class_file(second_arg_is_constant: bool) -> Vec<u8> {
        let mut c = vec![0xCA, 0xFE, 0xBA, 0xBE, 0, 0, 0, 69];
        let mut pool: Vec<Vec<u8>> = Vec::new();
        let utf8 = |s: &str| {
            let mut e = vec![1];
            e.extend((s.len() as u16).to_be_bytes());
            e.extend(s.as_bytes());
            e
        };
        let pair = |tag: u8, a: u16, b: u16| {
            let mut e = vec![tag];
            e.extend(a.to_be_bytes());
            e.extend(b.to_be_bytes());
            e
        };
        pool.push(utf8("T")); // 1
        pool.push(vec![7, 0, 1]); // 2: class T
        pool.push(utf8("reg")); // 3
        pool.push(utf8("(LX;)V")); // 4
        pool.push(pair(12, 3, 4)); // 5
        pool.push(pair(10, 2, 5)); // 6: Methodref T.reg
        pool.push(utf8("A")); // 7
        pool.push(utf8("LX;")); // 8
        pool.push(pair(12, 7, 8)); // 9
        pool.push(pair(9, 2, 9)); // 10: Fieldref T.A
        pool.push(utf8("B")); // 11
        pool.push(pair(12, 11, 8)); // 12
        pool.push(pair(9, 2, 12)); // 13: Fieldref T.B
        pool.push(utf8("<clinit>")); // 14
        pool.push(utf8("()V")); // 15
        pool.push(utf8("Code")); // 16
        pool.push(vec![5, 0, 0, 0, 0, 0, 0, 0, 1]); // 17 (+18): a long
        c.extend(19u16.to_be_bytes());
        for e in pool {
            c.extend(e);
        }
        c.extend([0, 0x21, 0, 2, 0, 0, 0, 0]); // flags, this, super, interfaces
        c.extend([0, 0]); // fields

        let mut code = vec![0xb2, 0, 10, 0xb8, 0, 6, 0x03]; // getstatic A; invokestatic reg; iconst_0
        code.push(0xaa); // tableswitch at pc 7: operands at 8
        code.extend(0i32.to_be_bytes()); // default
        code.extend(0i32.to_be_bytes()); // low
        code.extend(1i32.to_be_bytes()); // high
        code.extend([0u8; 8]); // two offsets
        if second_arg_is_constant {
            code.extend([0xb2, 0, 13]); // getstatic B
        } else {
            code.extend([0x01, 0x00, 0x00]); // aconst_null; nop; nop
        }
        code.extend([0xb8, 0, 6, 0xb1]); // invokestatic reg; return

        c.extend([0, 1]); // methods
        c.extend([0, 8, 0, 14, 0, 15, 0, 1]); // static <clinit>()V, 1 attribute
        c.extend([0, 16]);
        c.extend((12 + code.len() as u32).to_be_bytes());
        c.extend([0, 1, 0, 0]); // max_stack, max_locals
        c.extend((code.len() as u32).to_be_bytes());
        c.extend(&code);
        c.extend([0, 0, 0, 0]); // exception table, attributes
        c.extend([0, 0]); // class attributes
        c
    }

    #[test]
    fn reads_registration_order() {
        assert_eq!(
            registration_order(&class_file(true), "reg").unwrap(),
            ["A", "B"]
        );
    }

    #[test]
    fn fails_on_a_call_it_cannot_resolve() {
        assert!(registration_order(&class_file(false), "reg").is_err());
    }
}
