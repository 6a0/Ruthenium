# Contributing

## Developer Certificate of Origin

Every commit must be signed off (`git commit -s`), certifying the [Developer Certificate of Origin 1.1](https://developercertificate.org/). CI rejects commits without a `Signed-off-by:` line.

## No Mojang content, ever

Never commit textures, sounds, models, shaders, language files, jars, class files or decompiled source from Minecraft, in any form. Run `cargo xtask check-content` before pushing; CI runs it on every change. If the reference client jar is present locally, the check also compares every file byte-for-byte against it.

## Writing behaviour code (clean-room rules)

See [docs/reference-policy.md](docs/reference-policy.md). In short: the decompiled game is a behavioural reference. Describe the behaviour, then implement it in idiomatic Rust; never transliterate it.

## Code from other projects

- MIT, Apache-2.0, MPL-2.0, LGPL-3.0 and GPL-3.0 code may be reused with attribution, recorded in `PROVENANCE.md` when added.
- Never copy code from projects whose licence forbids it, for example Sodium (PolyForm Shield). Reimplement the ideas instead.
