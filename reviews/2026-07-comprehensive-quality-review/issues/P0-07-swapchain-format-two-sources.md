# P0-07: Swapchain format has two sources of truth — hardcoded `Bgra8UnormSrgb` vs `capabilities.formats[0]` (latent WASM/mobile black screen)

**Severity:** P1 (latent fatal validation error on the first-class WASM target) · **Area:** mandala/renderer · **Verified:** yes (code read; self-contradicting comment confirmed)

## Problem

`src/application/renderer/mod.rs:597-613`:

```rust
let swapchain_format = TextureFormat::Bgra8UnormSrgb;          // hardcoded
let surface_capabilities = surface.get_capabilities(&adapter);
let texture_format = surface_capabilities.formats[0];
let config = Self::create_surface_config(texture_format.clone(), ...);   // surface uses formats[0]
let mut atlas = TextAtlas::new(&device, &queue, &glyphon_cache, swapchain_format); // atlas uses hardcoded
```

The rect pipeline's color target also uses the hardcoded value (`pipeline.rs` / mod.rs:617-624), and the comment there is self-contradictory: "Uses the swapchain (not capability[0]) format so the pipeline matches the LoadOp target" — but the LoadOp target is the surface texture view, which was configured with `formats[0]`.

wgpu requires pipeline color-target format == render-pass attachment format; a mismatch is a fatal per-draw validation error (black screen / device loss), not a degraded frame. It works today only because `formats[0]` happens to be `Bgra8UnormSrgb` on the desktop backends tested. `Cargo.toml` enables wgpu's `webgl` feature, and the GL/WebGL2 backend commonly reports `Rgba8UnormSrgb` first — i.e. the WASM/mobile deployment (first-class per CODE_CONVENTIONS §4) is where the accidental agreement most plausibly breaks.

Minor: `texture_format.clone()` clones a `Copy` enum.

## Fix plan

1. Derive **one** format: `let format = surface_capabilities.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(surface_capabilities.formats[0]);` (or simply `formats[0]` if sRGB-first is guaranteed by the sort order — check wgpu docs for the pinned wgpu version).
2. Feed that single value to: surface config, `TextAtlas::new`, and the rect pipeline's `ColorTargetState`.
3. Delete the hardcoded constant and the self-contradicting comment; drop the `.clone()`.
4. Verification: `./build.sh` (native + wasm); run the WASM build in a browser (`trunk serve`) and confirm rendering — this is the only real end-to-end check since there are no GPU tests by policy (§T8).

## Acceptance criteria

- Exactly one format value flows to surface, atlas, and pipelines (grep for `Bgra8UnormSrgb` returns nothing in src/).
- Native + WASM render correctly.

## Pointers

`src/application/renderer/mod.rs:597-681`; `src/application/renderer/pipeline.rs:11-19`; CODE_CONVENTIONS §4 (mobile budget / cross-platform first-class); TEST_CONVENTIONS §T8 (no GPU tests — verify by running).
