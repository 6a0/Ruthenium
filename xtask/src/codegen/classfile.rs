//! Just enough of the JVM class-file format to walk a method's bytecode and
//! resolve the fields and methods it references.

/// `getstatic` and `invokestatic` opcodes.
pub const GETSTATIC: u8 = 0xb2;
pub const INVOKESTATIC: u8 = 0xb8;

#[derive(Debug)]
enum Entry {
    Utf8(String),
    Class(u16),
    /// Name index (the descriptor is not needed).
    NameAndType(u16),
    /// Fieldref, Methodref or InterfaceMethodref: class index, name-and-type index.
    Member(u16, u16),
    Other,
}

#[derive(Debug)]
pub struct Class {
    pool: Vec<Entry>,
    this_class: u16,
    /// (name, code) of each method that has a Code attribute.
    methods: Vec<(String, Vec<u8>)>,
}

/// A field or method reference, resolved through the constant pool.
#[derive(Debug, PartialEq, Eq)]
pub struct MemberRef<'a> {
    pub class: &'a str,
    pub name: &'a str,
}

#[derive(Debug)]
pub struct Instruction<'a> {
    pub opcode: u8,
    pub operands: &'a [u8],
}

impl Instruction<'_> {
    /// The constant-pool index of `getstatic`, `invokestatic` and friends.
    pub fn pool_index(&self) -> Option<u16> {
        match self.operands {
            [hi, lo, ..] => Some(u16::from_be_bytes([*hi, *lo])),
            _ => None,
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or("class file is truncated")?;
        let s = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
}

pub fn parse(bytes: &[u8]) -> Result<Class, String> {
    let mut r = Reader { bytes, pos: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return Err("not a class file".into());
    }
    r.take(4)?; // minor, major
    let count = r.u16()?;
    // Index 0 is unused; long and double take two slots.
    let mut pool = vec![Entry::Other];
    while pool.len() < count as usize {
        let tag = r.u8()?;
        let entry = match tag {
            1 => {
                let len = r.u16()? as usize;
                // Modified UTF-8; identical to UTF-8 for the ASCII names used here.
                Entry::Utf8(String::from_utf8_lossy(r.take(len)?).into_owned())
            }
            7 => Entry::Class(r.u16()?),
            9..=11 => Entry::Member(r.u16()?, r.u16()?),
            12 => {
                let name = r.u16()?;
                r.u16()?;
                Entry::NameAndType(name)
            }
            3 | 4 => {
                r.take(4)?;
                Entry::Other
            }
            5 | 6 => {
                r.take(8)?;
                pool.push(Entry::Other);
                Entry::Other
            }
            8 | 16 | 19 | 20 => {
                r.take(2)?;
                Entry::Other
            }
            15 => {
                r.take(3)?;
                Entry::Other
            }
            17 | 18 => {
                r.take(4)?;
                Entry::Other
            }
            t => return Err(format!("unknown constant-pool tag {t}")),
        };
        pool.push(entry);
    }
    r.u16()?; // access flags
    let this_class = r.u16()?;
    r.u16()?; // super class
    let interfaces = r.u16()? as usize;
    r.take(interfaces * 2)?;

    let mut class = Class {
        pool,
        this_class,
        methods: Vec::new(),
    };
    for is_method in [false, true] {
        let members = r.u16()?;
        for _ in 0..members {
            r.u16()?; // access flags
            let name = class.utf8(r.u16()?)?.to_owned();
            r.u16()?; // descriptor
            let attributes = r.u16()?;
            for _ in 0..attributes {
                let attr_name = r.u16()?;
                let len = r.u32()? as usize;
                let body = r.take(len)?;
                if is_method && class.utf8(attr_name)? == "Code" {
                    let mut c = Reader {
                        bytes: body,
                        pos: 4, // max_stack, max_locals
                    };
                    let code_len = c.u32()? as usize;
                    let code = c.take(code_len)?.to_vec();
                    class.methods.push((name.clone(), code));
                }
            }
        }
    }
    Ok(class)
}

impl Class {
    fn utf8(&self, index: u16) -> Result<&str, String> {
        match self.pool.get(index as usize) {
            Some(Entry::Utf8(s)) => Ok(s),
            _ => Err(format!("constant #{index} is not a string")),
        }
    }

    fn class_name(&self, index: u16) -> Result<&str, String> {
        match self.pool.get(index as usize) {
            Some(Entry::Class(name)) => self.utf8(*name),
            _ => Err(format!("constant #{index} is not a class")),
        }
    }

    /// This class's internal name, e.g. `net/minecraft/Foo`.
    pub fn name(&self) -> Result<&str, String> {
        self.class_name(self.this_class)
    }

    /// The bytecode of the first method with this name.
    pub fn code(&self, method: &str) -> Option<&[u8]> {
        self.methods
            .iter()
            .find(|(n, _)| n == method)
            .map(|(_, c)| c.as_slice())
    }

    /// Resolves a Fieldref or Methodref.
    pub fn member(&self, index: u16) -> Result<MemberRef<'_>, String> {
        let Some(Entry::Member(class, nat)) = self.pool.get(index as usize) else {
            return Err(format!("constant #{index} is not a member reference"));
        };
        let Some(Entry::NameAndType(name)) = self.pool.get(*nat as usize) else {
            return Err(format!("constant #{nat} is not a name-and-type"));
        };
        Ok(MemberRef {
            class: self.class_name(*class)?,
            name: self.utf8(*name)?,
        })
    }
}

/// Splits bytecode into instructions.
pub fn instructions(code: &[u8]) -> Result<Vec<Instruction<'_>>, String> {
    let mut out = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let opcode = code[pc];
        let int_at = |at: usize| -> Result<i32, String> {
            code.get(at..at + 4)
                .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                .ok_or_else(|| "bytecode is truncated".to_string())
        };
        // Switch operands start at the next multiple of 4 from the method start.
        let padded = (pc + 4) & !3;
        let len = match opcode {
            0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
            0x11
            | 0x13
            | 0x14
            | 0x84
            | 0x99..=0xa8
            | 0xb2..=0xb8
            | 0xbb
            | 0xbd
            | 0xc0
            | 0xc1
            | 0xc6
            | 0xc7 => 3,
            0xc5 => 4,
            0xb9 | 0xba | 0xc8 | 0xc9 => 5,
            0xc4 => {
                if code.get(pc + 1) == Some(&0x84) {
                    6
                } else {
                    4
                }
            }
            0xaa => {
                let (low, high) = (int_at(padded + 4)?, int_at(padded + 8)?);
                let n = usize::try_from(i64::from(high) - i64::from(low) + 1)
                    .map_err(|_| "bad tableswitch")?;
                padded + 12 + 4 * n - pc
            }
            0xab => {
                let n = usize::try_from(int_at(padded + 4)?).map_err(|_| "bad lookupswitch")?;
                padded + 8 + 8 * n - pc
            }
            0x00..=0x0f
            | 0x1a..=0x35
            | 0x3b..=0x83
            | 0x85..=0x98
            | 0xac..=0xb1
            | 0xbe
            | 0xbf
            | 0xc2
            | 0xc3 => 1,
            op => return Err(format!("unknown opcode 0x{op:02x} at {pc}")),
        };
        let operands = code.get(pc + 1..pc + len).ok_or("bytecode is truncated")?;
        out.push(Instruction { opcode, operands });
        pc += len;
    }
    Ok(out)
}
