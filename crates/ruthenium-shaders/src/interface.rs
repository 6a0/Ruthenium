//! What the world's shaders read from the frame's data: records of 32-bit words, each
//! found by a word offset passed in push constants.
//!
//! The frame's data is one buffer that several kinds of record share in a frame (GUI
//! quads, the world's values, a matrix, one small record per section drawn). The menus'
//! shaders each declare it as an array of their one element type; a world shader reads
//! several kinds in one draw, so it declares the buffer as plain words and reads fields
//! through the accessors that `cargo xtask shaders` writes into
//! `glsl/include/world.glsl` from the tables below. The renderer writes the same
//! fields at the same offsets ([`Record::offset`]), so the two sides cannot drift apart
//! without a test noticing (the compiled shaders are also read back and compared).
//!
//! This file is built into `ruthenium-shaders` and included as source by the xtask,
//! which must not depend on the compiled shaders it is about to write: std only.

/// The type of a field, as GLSL names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    Uint,
    Float,
    Vec2,
    Vec3,
    Vec4,
    IVec3,
    Mat4,
}

impl Ty {
    /// How many words a field of the type takes.
    pub const fn words(self) -> u32 {
        match self {
            Ty::Uint | Ty::Float => 1,
            Ty::Vec2 => 2,
            Ty::Vec3 | Ty::IVec3 => 3,
            Ty::Vec4 => 4,
            Ty::Mat4 => 16,
        }
    }

    pub const fn glsl(self) -> &'static str {
        match self {
            Ty::Uint => "uint",
            Ty::Float => "float",
            Ty::Vec2 => "vec2",
            Ty::Vec3 => "vec3",
            Ty::Vec4 => "vec4",
            Ty::IVec3 => "ivec3",
            Ty::Mat4 => "mat4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub ty: Ty,
    pub doc: &'static str,
}

/// One kind of record: its fields follow each other without gaps, a word at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    /// What its accessors and its size constant are named after.
    pub name: &'static str,
    pub doc: &'static str,
    pub fields: &'static [Field],
}

const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

impl Record {
    /// Words in one record.
    pub const fn words(&self) -> u32 {
        let mut words = 0;
        let mut i = 0;
        while i < self.fields.len() {
            words += self.fields[i].ty.words();
            i += 1;
        }
        words
    }

    /// Bytes in one record.
    pub const fn bytes(&self) -> u32 {
        self.words() * 4
    }

    /// The word a field starts at.
    ///
    /// # Panics
    /// If the record has no such field (at compile time, used in a constant).
    pub const fn offset(&self, field: &str) -> u32 {
        let mut words = 0;
        let mut i = 0;
        while i < self.fields.len() {
            if same(self.fields[i].name, field) {
                return words;
            }
            words += self.fields[i].ty.words();
            i += 1;
        }
        panic!("no such field");
    }
}

/// Descriptor set and binding of the frame's data (`ruthenium-gpu`'s `FRAME_DATA_SET`).
pub const FRAME_DATA_SET: u32 = 1;
pub const FRAME_DATA_BINDING: u32 = 0;

/// What every terrain draw of a frame reads in its vertex stage. (What the fragment
/// stages read is in push constants: read from this buffer for every pixel, the same
/// few values cost a fifth of a frame's GPU time.)
pub const WORLD: Record = Record {
    name: "world",
    doc: "What every terrain vertex of a frame shares.",
    fields: &[
        Field {
            name: "camera_offset",
            ty: Ty::Vec3,
            doc: "Minus the camera's place inside its block: positions are taken relative to the camera's block in whole numbers first.",
        },
        Field {
            name: "atlas_size",
            ty: Ty::Vec2,
            doc: "The atlas's size in texels.",
        },
        Field {
            name: "lightmap",
            ty: Ty::Uint,
            doc: "The 16 x 16 light map, an index into the texture table.",
        },
    ],
};

/// The camera's matrix, a record of its own.
pub const WORLD_MATRIX: Record = Record {
    name: "world_matrix",
    doc: "The camera's matrix.",
    fields: &[Field {
        name: "view_projection",
        ty: Ty::Mat4,
        doc: "From blocks relative to the camera to clip space, columns first.",
    }],
};

/// What one section's draws read: found by the draw's instance number.
pub const SECTION: Record = Record {
    name: "section",
    doc: "One section's draw: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "origin",
            ty: Ty::IVec3,
            doc: "The section's corner in blocks from the camera's block.",
        },
        Field {
            name: "visibility",
            ty: Ty::Float,
            doc: "How far the section has faded in, 0 to 1.",
        },
    ],
};

/// What one draw of the sky reads in its vertex stage.
pub const SKY: Record = Record {
    name: "sky",
    doc: "One draw of the sky: the dome, the glow of a sunrise, the sun, the moon, the stars.",
    fields: &[Field {
        name: "transform",
        ty: Ty::Mat4,
        doc: "From the draw's own space, in blocks around the camera, to clip space, columns first.",
    }],
};

/// What one part of a drawn model reads: found by the instance number.
pub const MODEL_PART: Record = Record {
    name: "model_part",
    doc: "One part of a model: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "row_x",
            ty: Ty::Vec4,
            doc: "From the part's own space to blocks relative to the camera: the first row.",
        },
        Field {
            name: "row_y",
            ty: Ty::Vec4,
            doc: "The second row.",
        },
        Field {
            name: "row_z",
            ty: Ty::Vec4,
            doc: "The third row.",
        },
        Field {
            name: "light",
            ty: Ty::Uint,
            doc: "Block light in the low byte and sky light in the next, each 0 to 240.",
        },
        Field {
            name: "colour",
            ty: Ty::Uint,
            doc: "What the texture is multiplied by: red in the low byte, then green, blue and alpha.",
        },
        Field {
            name: "texture",
            ty: Ty::Uint,
            doc: "An index into the texture table.",
        },
        Field {
            name: "overlay",
            ty: Ty::Uint,
            doc: "The lowest bit: it is tinted red, hurt. From the ninth bit up: the word offset of the GLINT record of what glints over it, 0 for nothing.",
        },
    ],
};

/// What the glyphs of one side of a sign share.
pub const SIGN: Record = Record {
    name: "sign",
    doc: "One side of a sign: found by its glyphs.",
    fields: &[
        Field {
            name: "row_x",
            ty: Ty::Vec4,
            doc: "From the text's own space (y runs down) to blocks relative to the camera: the first row.",
        },
        Field {
            name: "row_y",
            ty: Ty::Vec4,
            doc: "The second row.",
        },
        Field {
            name: "row_z",
            ty: Ty::Vec4,
            doc: "The third row.",
        },
        Field {
            name: "light",
            ty: Ty::Uint,
            doc: "Block light in the low byte and sky light in the next, each 0 to 240; with the top bit set it is drawn as it is, in no light.",
        },
        Field {
            name: "outline",
            ty: Ty::Uint,
            doc: "The colour of what surrounds its glyphs: red in the low byte, then green, blue and alpha.",
        },
    ],
};

/// What one glyph on a sign reads: found by the instance number.
pub const GLYPH: Record = Record {
    name: "glyph",
    doc: "One glyph on a sign: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "box",
            ty: Ty::Vec4,
            doc: "Its left, top, right and bottom in the text's space.",
        },
        Field {
            name: "slant",
            ty: Ty::Vec2,
            doc: "How far its top and its bottom are moved to the right.",
        },
        Field {
            name: "nearer",
            ty: Ty::Float,
            doc: "How far in front of the text it lies.",
        },
        Field {
            name: "texels",
            ty: Ty::Uint,
            doc: "Its picture's first column in the low half and first row in the high one.",
        },
        Field {
            name: "size",
            ty: Ty::Uint,
            doc: "Its picture's width in the low half and height in the high one.",
        },
        Field {
            name: "colour",
            ty: Ty::Uint,
            doc: "What the picture is multiplied by: red in the low byte, then green, blue and alpha.",
        },
        Field {
            name: "texture",
            ty: Ty::Uint,
            doc: "An index into the texture table.",
        },
        Field {
            name: "sign",
            ty: Ty::Uint,
            doc: "The number of its SIGN record.",
        },
        Field {
            name: "around",
            ty: Ty::Float,
            doc: "How far from it its outline lies; nothing for one that has none.",
        },
    ],
};

/// What one piece of an entity's shadow reads: the part of it that lies on one block.
pub const SHADOW: Record = Record {
    name: "shadow",
    doc: "One block's piece of an entity's shadow: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "origin",
            ty: Ty::Vec3,
            doc: "Where its entity stands, in blocks relative to the camera.",
        },
        Field {
            name: "radius",
            ty: Ty::Float,
            doc: "How far the whole shadow reaches from there.",
        },
        Field {
            name: "box",
            ty: Ty::Vec4,
            doc: "The piece's west, north, east and south edges from there.",
        },
        Field {
            name: "height",
            ty: Ty::Float,
            doc: "How far above there it lies.",
        },
        Field {
            name: "alpha",
            ty: Ty::Float,
            doc: "How dark it is, 0 to 1.",
        },
    ],
};

/// What glints over enchanted things: one record for each scale it is drawn at.
pub const GLINT: Record = Record {
    name: "glint",
    doc: "The glint over what is enchanted: found by the parts it lies over.",
    fields: &[
        Field {
            name: "texture",
            ty: Ty::Uint,
            doc: "Its picture, an index into the texture table.",
        },
        Field {
            name: "strength",
            ty: Ty::Float,
            doc: "How strong it is, 0 to 1.",
        },
        Field {
            name: "row_u",
            ty: Ty::Vec3,
            doc: "From a part's texture coordinates (and 1) to where its picture is read: the first row.",
        },
        Field {
            name: "row_v",
            ty: Ty::Vec3,
            doc: "The second row.",
        },
    ],
};

/// One edge of the outline of the block the crosshair is on.
pub const LINE: Record = Record {
    name: "line",
    doc: "One line drawn as wide on the screen wherever it is: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "from",
            ty: Ty::Vec3,
            doc: "One end, in blocks relative to the camera.",
        },
        Field {
            name: "width",
            ty: Ty::Float,
            doc: "How wide it is, in pixels of the target.",
        },
        Field {
            name: "to",
            ty: Ty::Vec3,
            doc: "The other end.",
        },
        Field {
            name: "colour",
            ty: Ty::Uint,
            doc: "Its colour: red in the low byte, then green, blue and alpha.",
        },
    ],
};

/// One face of a block that is being broken, with the cracks laid over it.
pub const CRACK: Record = Record {
    name: "crack",
    doc: "One quad of the cracks in a block being broken: record number gl_InstanceIndex from the offset in push constants.",
    fields: &[
        Field {
            name: "a",
            ty: Ty::Vec3,
            doc: "Its first corner, in blocks from the camera's block; the quad is the triangles a-b-c and a-c-d.",
        },
        Field {
            name: "b",
            ty: Ty::Vec3,
            doc: "The second.",
        },
        Field {
            name: "c",
            ty: Ty::Vec3,
            doc: "The third.",
        },
        Field {
            name: "d",
            ty: Ty::Vec3,
            doc: "The fourth.",
        },
        Field {
            name: "uv_a",
            ty: Ty::Vec2,
            doc: "Where the cracks' picture is read at the first corner, in pictures (it repeats).",
        },
        Field {
            name: "uv_b",
            ty: Ty::Vec2,
            doc: "At the second.",
        },
        Field {
            name: "uv_c",
            ty: Ty::Vec2,
            doc: "At the third.",
        },
        Field {
            name: "uv_d",
            ty: Ty::Vec2,
            doc: "At the fourth.",
        },
    ],
};

/// Every record `world.glsl` has accessors for.
pub const RECORDS: [&Record; 11] = [
    &WORLD,
    &WORLD_MATRIX,
    &SECTION,
    &SKY,
    &MODEL_PART,
    &SIGN,
    &GLYPH,
    &SHADOW,
    &GLINT,
    &LINE,
    &CRACK,
];

/// The text of `glsl/include/world.glsl`.
pub fn world_glsl() -> String {
    let mut s = String::from(
        "// Ruthenium: the frame's data as the world's shaders read it: records of words, each found\n\
         // by a word offset from push constants.\n\
         //\n\
         // @generated by `cargo xtask shaders` from crates/ruthenium-shaders/src/interface.rs.\n\
         // Do not edit by hand.\n",
    );
    s += &format!(
        "\nlayout(set = {FRAME_DATA_SET}, binding = {FRAME_DATA_BINDING}, std430) readonly buffer FrameWords {{\n    uint frame_words[];\n}};\n"
    );
    for r in RECORDS {
        s += &format!(
            "\n// {}\n#define {}_WORDS {}u\n",
            r.doc,
            r.name.to_uppercase(),
            r.words()
        );
        let mut at = 0;
        for f in r.fields {
            let word = |i: u32| format!("frame_words[at + {}u]", at + i);
            let float = |i: u32| format!("uintBitsToFloat({})", word(i));
            let list =
                |n: u32, of: &dyn Fn(u32) -> String| (0..n).map(of).collect::<Vec<_>>().join(", ");
            let value = match f.ty {
                Ty::Uint => word(0),
                Ty::Float => float(0),
                Ty::Vec2 | Ty::Vec3 | Ty::Vec4 => {
                    format!("{}({})", f.ty.glsl(), list(f.ty.words(), &float))
                }
                Ty::IVec3 => format!("ivec3({})", list(3, &|i| format!("int({})", word(i)))),
                Ty::Mat4 => format!(
                    "mat4(\n        {})",
                    (0..4)
                        .map(|c| list(4, &|i| float(c * 4 + i)))
                        .collect::<Vec<_>>()
                        .join(",\n        ")
                ),
            };
            s += &format!(
                "\n// {}\n{} {}_{}(uint at) {{\n    return {value};\n}}\n",
                f.doc,
                f.ty.glsl(),
                r.name,
                f.name
            );
            at += f.ty.words();
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_follow_each_other_a_word_at_a_time() {
        assert_eq!(WORLD.offset("camera_offset"), 0);
        assert_eq!(WORLD.offset("atlas_size"), 3);
        assert_eq!(WORLD.offset("lightmap"), 5);
        assert_eq!(WORLD.words(), 6);
        assert_eq!(SKY.offset("transform"), 0);
        assert_eq!(SKY.bytes(), 64);
        assert_eq!(WORLD_MATRIX.bytes(), 64);
        assert_eq!(SECTION.offset("visibility"), 3);
        assert_eq!(SECTION.bytes(), 16);
        assert_eq!(MODEL_PART.offset("light"), 12);
        assert_eq!((MODEL_PART.offset("overlay"), MODEL_PART.bytes()), (15, 64));
        assert_eq!((SIGN.offset("light"), SIGN.words()), (12, 14));
        assert_eq!((GLYPH.offset("sign"), GLYPH.words()), (11, 13));
        assert_eq!((SHADOW.offset("box"), SHADOW.offset("alpha")), (4, 9));
        assert_eq!(SHADOW.bytes(), 40);
        assert_eq!((GLINT.offset("row_u"), GLINT.words()), (2, 8));
        for r in RECORDS {
            let mut names: Vec<&str> = r.fields.iter().map(|f| f.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), r.fields.len(), "{}", r.name);
        }
    }

    #[test]
    fn the_committed_accessors_are_the_generated_ones() {
        let committed = include_str!("../glsl/include/world.glsl");
        assert_eq!(committed, world_glsl(), "run `cargo xtask shaders`");
        // One accessor per field, reading the words its offset says.
        assert!(
            committed.contains("uint world_lightmap(uint at) {\n    return frame_words[at + 5u];")
        );
        assert!(committed.contains("#define SECTION_WORDS 4u"));
    }
}
