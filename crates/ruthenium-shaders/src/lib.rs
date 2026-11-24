//! Ruthenium's shaders, as SPIR-V compiled offline from `glsl/` by `cargo xtask shaders`
//! (a pinned glslc; CI fails if the committed SPIR-V is stale). Nothing is compiled at
//! build time or at run time.

mod generated;
pub mod interface;

pub use generated::*;

/// Bytes of the one push-constant range every pipeline layout has (`glsl/include/push.glsl`).
pub const PUSH_CONSTANT_BYTES: u32 = 128;

/// Storage for `include_bytes!` with SPIR-V's 4-byte alignment.
#[doc(hidden)]
#[repr(C, align(4))]
pub struct Aligned<T: ?Sized>(pub T);

impl<T: ?Sized> std::fmt::Debug for Aligned<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Aligned(..)")
    }
}

/// One compiled shader stage.
pub struct Spirv {
    /// Source file name, e.g. `background.frag`.
    pub name: &'static str,
    #[doc(hidden)]
    pub bytes: &'static Aligned<[u8]>,
}

impl std::fmt::Debug for Spirv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Spirv")
            .field("name", &self.name)
            .field("bytes", &self.bytes.0.len())
            .finish()
    }
}

impl Spirv {
    /// The module as 32-bit words, as `vkCreateShaderModule` takes it.
    pub fn words(&self) -> &'static [u32] {
        let bytes = &self.bytes.0;
        assert!(
            bytes.len().is_multiple_of(4),
            "{}: SPIR-V length is not a multiple of 4",
            self.name
        );
        // SAFETY: `Aligned` gives the bytes 4-byte alignment, the length is a multiple of 4,
        // and every bit pattern is a valid u32.
        unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u32>(), bytes.len() / 4) }
    }
}

#[doc(hidden)]
#[macro_export]
macro_rules! spirv {
    ($name:literal) => {
        $crate::Spirv {
            name: $name,
            bytes: &$crate::Aligned(*include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/spirv/",
                $name,
                ".spv"
            ))),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_is_spirv() {
        assert!(!ALL.is_empty());
        for s in ALL {
            let w = s.words();
            assert_eq!(w[0], 0x0723_0203, "{}: bad SPIR-V magic", s.name);
            // Version word: SPIR-V 1.5 at most (Vulkan 1.2).
            assert!(
                w[1] <= 0x0001_0500,
                "{}: SPIR-V version {:#x}",
                s.name,
                w[1]
            );
        }
    }
}
