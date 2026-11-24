//! Reads a compiled shader's interface back out of its SPIR-V, for tests: the vertex
//! inputs, the push-constant block, and which descriptor sets and bindings it uses with
//! what. The renderer's own descriptions of those (the vertex layout, the bytes it
//! pushes, the frame data's set) are checked against what the shaders really declare.
//!
//! The shaders are compiled without names, so everything is found by what the SPIR-V
//! itself says: decorations, storage classes and types.

use std::collections::HashMap;

/// A type as far as the checks tell types apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Type {
    Uint,
    Int,
    Float,
    /// Component type and count.
    Vector(Box<Type>, u32),
    Matrix(Box<Type>, u32),
    /// Member types with their byte offsets.
    Struct(Vec<(Type, Option<u32>)>),
    /// Element type and its stride in bytes, where one is declared.
    RuntimeArray(Box<Type>, Option<u32>),
    Array(Box<Type>, u32),
    Image,
    Sampler,
    Other,
}

/// A variable of a shader's interface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Variable {
    pub ty: Type,
    pub location: Option<u32>,
    pub set: Option<u32>,
    pub binding: Option<u32>,
    pub builtin: bool,
}

/// A shader's interface, by where its variables live.
#[derive(Debug, Default)]
pub(crate) struct Interface {
    pub inputs: Vec<Variable>,
    pub outputs: Vec<Variable>,
    pub push_constants: Vec<Variable>,
    /// Uniforms, storage buffers, textures and samplers.
    pub resources: Vec<Variable>,
}

const OP_TYPE_INT: u32 = 21;
const OP_TYPE_FLOAT: u32 = 22;
const OP_TYPE_VECTOR: u32 = 23;
const OP_TYPE_MATRIX: u32 = 24;
const OP_TYPE_IMAGE: u32 = 25;
const OP_TYPE_SAMPLER: u32 = 26;
const OP_TYPE_ARRAY: u32 = 28;
const OP_TYPE_RUNTIME_ARRAY: u32 = 29;
const OP_TYPE_STRUCT: u32 = 30;
const OP_TYPE_POINTER: u32 = 32;
const OP_CONSTANT: u32 = 43;
const OP_VARIABLE: u32 = 59;
const OP_DECORATE: u32 = 71;
const OP_MEMBER_DECORATE: u32 = 72;

const DECORATION_ARRAY_STRIDE: u32 = 6;
const DECORATION_BUILTIN: u32 = 11;
const DECORATION_LOCATION: u32 = 30;
const DECORATION_BINDING: u32 = 33;
const DECORATION_DESCRIPTOR_SET: u32 = 34;
const DECORATION_OFFSET: u32 = 35;

const STORAGE_UNIFORM_CONSTANT: u32 = 0;
const STORAGE_INPUT: u32 = 1;
const STORAGE_UNIFORM: u32 = 2;
const STORAGE_OUTPUT: u32 = 3;
const STORAGE_PUSH_CONSTANT: u32 = 9;
const STORAGE_BUFFER: u32 = 12;

/// Reads the interface of a SPIR-V module.
pub(crate) fn interface(words: &[u32]) -> Interface {
    assert_eq!(words[0], 0x0723_0203, "SPIR-V magic");
    let mut instructions = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let (count, op) = ((words[at] >> 16) as usize, words[at] & 0xFFFF);
        assert!(
            count > 0 && at + count <= words.len(),
            "a broken instruction"
        );
        instructions.push((op, &words[at + 1..at + count]));
        at += count;
    }
    // Decorations first: they come before the types they decorate are used.
    let mut decorations: HashMap<(u32, u32), u32> = HashMap::new();
    let mut builtins: Vec<u32> = Vec::new();
    let mut offsets: HashMap<(u32, u32), u32> = HashMap::new();
    for (op, operands) in &instructions {
        match *op {
            OP_DECORATE if operands[1] == DECORATION_BUILTIN => builtins.push(operands[0]),
            OP_DECORATE if operands.len() > 2 => {
                decorations.insert((operands[0], operands[1]), operands[2]);
            }
            OP_MEMBER_DECORATE if operands[2] == DECORATION_OFFSET => {
                offsets.insert((operands[0], operands[1]), operands[3]);
            }
            _ => {}
        }
    }
    let mut types: HashMap<u32, Type> = HashMap::new();
    let mut pointers: HashMap<u32, u32> = HashMap::new();
    let mut constants: HashMap<u32, u32> = HashMap::new();
    let mut out = Interface::default();
    for (op, o) in &instructions {
        let of =
            |id: u32, types: &HashMap<u32, Type>| types.get(&id).cloned().unwrap_or(Type::Other);
        match *op {
            OP_TYPE_INT => {
                types.insert(o[0], if o[2] == 0 { Type::Uint } else { Type::Int });
            }
            OP_TYPE_FLOAT => {
                types.insert(o[0], Type::Float);
            }
            OP_TYPE_VECTOR => {
                types.insert(o[0], Type::Vector(Box::new(of(o[1], &types)), o[2]));
            }
            OP_TYPE_MATRIX => {
                types.insert(o[0], Type::Matrix(Box::new(of(o[1], &types)), o[2]));
            }
            OP_TYPE_IMAGE => {
                types.insert(o[0], Type::Image);
            }
            OP_TYPE_SAMPLER => {
                types.insert(o[0], Type::Sampler);
            }
            OP_TYPE_ARRAY => {
                let length = constants.get(&o[2]).copied().unwrap_or(0);
                types.insert(o[0], Type::Array(Box::new(of(o[1], &types)), length));
            }
            OP_TYPE_RUNTIME_ARRAY => {
                let stride = decorations.get(&(o[0], DECORATION_ARRAY_STRIDE)).copied();
                types.insert(o[0], Type::RuntimeArray(Box::new(of(o[1], &types)), stride));
            }
            OP_TYPE_STRUCT => {
                let members = o[1..]
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (of(*m, &types), offsets.get(&(o[0], i as u32)).copied()))
                    .collect();
                types.insert(o[0], Type::Struct(members));
            }
            OP_TYPE_POINTER => {
                pointers.insert(o[0], o[2]);
            }
            OP_CONSTANT if o.len() == 3 => {
                constants.insert(o[1], o[2]);
            }
            OP_VARIABLE => {
                let (pointer, id, storage) = (o[0], o[1], o[2]);
                let variable = Variable {
                    ty: pointers
                        .get(&pointer)
                        .map_or(Type::Other, |t| of(*t, &types)),
                    location: decorations.get(&(id, DECORATION_LOCATION)).copied(),
                    set: decorations.get(&(id, DECORATION_DESCRIPTOR_SET)).copied(),
                    binding: decorations.get(&(id, DECORATION_BINDING)).copied(),
                    builtin: builtins.contains(&id),
                };
                match storage {
                    STORAGE_INPUT => out.inputs.push(variable),
                    STORAGE_OUTPUT => out.outputs.push(variable),
                    STORAGE_PUSH_CONSTANT => out.push_constants.push(variable),
                    STORAGE_UNIFORM | STORAGE_BUFFER | STORAGE_UNIFORM_CONSTANT => {
                        out.resources.push(variable);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    out
}

impl Interface {
    /// The inputs (or outputs) with a location, in its order.
    pub fn located(variables: &[Variable]) -> Vec<(u32, &Type)> {
        let mut v: Vec<(u32, &Type)> = variables
            .iter()
            .filter(|v| !v.builtin)
            .filter_map(|v| Some((v.location?, &v.ty)))
            .collect();
        v.sort_by_key(|(l, _)| *l);
        v
    }

    /// The resource at a descriptor set and binding.
    pub fn resource(&self, set: u32, binding: u32) -> Option<&Type> {
        self.resources
            .iter()
            .find(|v| v.set == Some(set) && v.binding == Some(binding))
            .map(|v| &v.ty)
    }
}

#[cfg(test)]
mod tests {
    use ruthenium_gpu::vk;
    use ruthenium_shaders as shaders;
    use ruthenium_shaders::interface::{FRAME_DATA_BINDING, FRAME_DATA_SET};

    use super::*;
    use crate::terrain::world::terrain_push;
    use crate::terrain_vertex::TERRAIN_VERTEX;

    /// What a shader sees of a vertex attribute of a format: unsigned whole numbers or
    /// fractions, and how many.
    fn seen(format: vk::Format) -> Type {
        match format {
            vk::Format::R16G16B16A16_UINT => Type::Vector(Box::new(Type::Uint), 4),
            vk::Format::R16G16_UINT => Type::Vector(Box::new(Type::Uint), 2),
            vk::Format::R8G8B8A8_UNORM => Type::Vector(Box::new(Type::Float), 4),
            other => panic!("no shader type known for {other:?}"),
        }
    }

    fn vec(n: u32) -> Type {
        Type::Vector(Box::new(Type::Float), n)
    }

    #[test]
    fn the_terrain_vertex_layout_is_what_the_shader_reads() {
        let vert = interface(shaders::TERRAIN_VERT.words());
        let inputs = Interface::located(&vert.inputs);
        assert_eq!(inputs.len(), TERRAIN_VERTEX.attributes.len());
        for (i, attribute) in TERRAIN_VERTEX.attributes.iter().enumerate() {
            assert_eq!(inputs[i].0, i as u32, "locations are the attributes' order");
            assert_eq!(*inputs[i].1, seen(attribute.format), "location {i}");
        }
        // The attributes lie inside the vertex and do not overlap.
        let mut end = 0;
        for a in TERRAIN_VERTEX.attributes {
            assert!(a.offset >= end);
            end = a.offset
                + match a.format {
                    vk::Format::R16G16B16A16_UINT => 8,
                    _ => 4,
                };
        }
        assert_eq!(end, TERRAIN_VERTEX.stride);
    }

    #[test]
    fn the_stages_agree_on_what_passes_between_them() {
        let vert = interface(shaders::TERRAIN_VERT.words());
        let outputs = Interface::located(&vert.outputs);
        assert_eq!(outputs, vec![(0, &vec(4)), (1, &vec(2))]);
        for frag in [&shaders::TERRAIN_FRAG, &shaders::TERRAIN_CUTOUT_FRAG] {
            let frag = interface(frag.words());
            assert_eq!(Interface::located(&frag.inputs), outputs);
            assert_eq!(Interface::located(&frag.outputs), vec![(0, &vec(4))]);
        }
    }

    #[test]
    fn push_constants_are_four_words_in_the_order_they_are_written() {
        // What the renderer pushes: four words, the last one the draw's "enabled".
        let pushed = terrain_push(1, 2, 3);
        assert_eq!(pushed.len(), 16);
        assert!(pushed.len() <= ruthenium_gpu::PUSH_CONSTANT_BYTES as usize);
        let vert = interface(shaders::TERRAIN_VERT.words());
        assert_eq!(vert.push_constants.len(), 1);
        // The vertex stage reads all four.
        assert_eq!(
            vert.push_constants[0].ty,
            Type::Struct(vec![
                (Type::Uint, Some(0)),
                (Type::Uint, Some(4)),
                (Type::Uint, Some(8)),
                (Type::Uint, Some(12)),
            ])
        );
        // A fragment stage declares the same block; what it keeps of it after
        // optimisation is words at offsets the renderer writes.
        for frag in [&shaders::TERRAIN_FRAG, &shaders::TERRAIN_CUTOUT_FRAG] {
            let frag = interface(frag.words());
            assert_eq!(frag.push_constants.len(), 1);
            let Type::Struct(members) = &frag.push_constants[0].ty else {
                panic!("the push constants are a block");
            };
            assert!(!members.is_empty());
            for (ty, offset) in members {
                assert_eq!(*ty, Type::Uint);
                assert!(matches!(offset, Some(0 | 4 | 8 | 12)), "{offset:?}");
            }
        }
    }

    #[test]
    fn the_frames_data_is_words_at_its_set_and_binding() {
        assert_eq!(FRAME_DATA_SET, ruthenium_gpu::FRAME_DATA_SET);
        let words = Type::Struct(vec![(
            Type::RuntimeArray(Box::new(Type::Uint), Some(4)),
            Some(0),
        )]);
        for shader in [
            &shaders::TERRAIN_VERT,
            &shaders::TERRAIN_FRAG,
            &shaders::TERRAIN_CUTOUT_FRAG,
        ] {
            let i = interface(shader.words());
            assert_eq!(
                i.resource(FRAME_DATA_SET, FRAME_DATA_BINDING),
                Some(&words),
                "{}",
                shader.name
            );
            // The texture table: samplers at binding 0, textures at binding 1 of set 0.
            assert_eq!(
                i.resource(0, 0),
                Some(&Type::Array(
                    Box::new(Type::Sampler),
                    ruthenium_gpu::SAMPLER_COUNT
                )),
                "{}",
                shader.name
            );
            assert_eq!(
                i.resource(0, 1),
                Some(&Type::RuntimeArray(Box::new(Type::Image), None)),
                "{}",
                shader.name
            );
            // Nothing else is bound.
            assert_eq!(i.resources.len(), 3, "{}", shader.name);
        }
    }

    #[test]
    fn the_menus_shaders_read_back_too() {
        // The reader is not only right about the shaders it was written for: the GUI's
        // vertex stage has no vertex inputs, 16 bytes of push constants, and its quads
        // in the frame's data.
        let gui = interface(shaders::GUI_VERT.words());
        assert!(Interface::located(&gui.inputs).is_empty());
        let Type::Struct(push) = &gui.push_constants[0].ty else {
            panic!("a block");
        };
        assert_eq!(push.last().unwrap().1, Some(12));
        let Some(Type::Struct(data)) = gui.resource(FRAME_DATA_SET, FRAME_DATA_BINDING) else {
            panic!("the frame's data");
        };
        let Type::RuntimeArray(_, stride) = &data[0].0 else {
            panic!("an array of quads");
        };
        assert_eq!(*stride, Some(crate::gui::QUAD_BYTES));
    }
}
