// SPDX-License-Identifier: MPL-2.0

//! Once-per-process surface setup. The adapter and the device it is
//! configured against are `adapter`'s.

use wgpu::{SurfaceCapabilities, SurfaceConfiguration, TextureFormat};
use winit::dpi::PhysicalSize;

use super::Renderer;

impl Renderer {
    pub(super) fn create_surface_config(
        texture_format: TextureFormat,
        surface_capabilities: &SurfaceCapabilities,
        surface_size: PhysicalSize<u32>,
    ) -> SurfaceConfiguration {
        SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: texture_format,
            // `Auto` is wgpu 30's default and reproduces the color
            // space every earlier wgpu picked implicitly for the
            // chosen format: sRGB for the sRGB format
            // `Renderer::for_window` prefers, and the historical choice
            // for its `formats[0]` fallback. The canvas renders as it did before the field
            // existed.
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: surface_size.width,
            height: surface_size.height,
            present_mode: wgpu::PresentMode::Fifo,
            // Interactive-first latency budget. wgpu's default is 2,
            // which at 60Hz bakes ~33ms of input-to-photon queueing
            // into every frame (and ~12ms at 165Hz — the user-visible
            // asymmetry that made rapid drags feel laggy on 60Hz
            // monitors while feeling fine on 165Hz). A single queued
            // frame still lets the GPU overlap with the CPU but caps
            // the backlog at one refresh interval.
            desired_maximum_frame_latency: 1,
            alpha_mode: surface_capabilities.alpha_modes[0],
            view_formats: vec![],
        }
    }
}
