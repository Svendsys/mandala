// SPDX-License-Identifier: MPL-2.0

//! Which GPU adapter the renderer runs on, and the order it tries them
//! in.
//!
//! **One way in, for every driver.** The window, the browser canvas and
//! the headless target all reach a device through [`acquire`]: wgpu's
//! own `request_adapter`, asked once per [`Attempt`] of the plan
//! [`attempts`] lays out, with the window's surface as the compatibility
//! constraint when there is one. `request_adapter` stays the mechanism
//! rather than a ranking of our own over `enumerate_adapters` because
//! it is the one that sees the surface: a GL adapter is discovered
//! through the surface it will present to, and a re-ranking of a
//! surface-less enumeration could lose the only adapter a GL-only
//! desktop has.
//!
//! **The plan.** Under [`AdapterPolicy::Auto`] the first attempt asks
//! for the adapter the power preference favors, among every adapter
//! the instance exposes — software ones included, ranked last — so on
//! a machine with nothing but Mesa's lavapipe or llvmpipe the first
//! attempt already lands on it. The second asks for a software adapter
//! only (`force_fallback_adapter`), and is the plan's answer to a
//! hardware adapter that is listed but cannot open a device: it runs
//! only when the first failed somewhere other than on a software
//! adapter ([`worth_retrying`]), since asking for one again would hand
//! back the adapter that just failed. [`AdapterPolicy::Software`] — the
//! `--adapter software` flag — is the second attempt alone: CI wanting
//! the same rasterizer on every runner, whatever GPU one happens to
//! carry.
//!
//! **The environment is honored, and only where wgpu names it.**
//! `WGPU_BACKEND` narrows the instance's backends
//! ([`instance_descriptor`]), `WGPU_POWER_PREF` sets the preference the
//! attempts carry (`high` when unset, which is what the renderer always
//! asked for), and `WGPU_ADAPTER_NAME` picks an adapter by name
//! ([`match_name`]) ahead of the plan. Each is wgpu's own variable with
//! wgpu's own meaning; with none set, the adapter is the one the
//! renderer has always asked for.

use wgpu::{Adapter, Device, DeviceType, Instance, PowerPreference, Queue, Surface};

/// Which adapters the renderer may run on: the `--adapter` flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterPolicy {
    /// The adapter the power preference favors, falling back to a
    /// software one ([`attempts`]).
    #[default]
    Auto,
    /// A software (CPU) adapter only — llvmpipe, lavapipe, WARP.
    // Native-only producer: the flag; the browser has no command line.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Software,
}

/// One `request_adapter` the plan makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Attempt {
    pub(super) power_preference: PowerPreference,
    /// `force_fallback_adapter`: only a software adapter qualifies.
    pub(super) software_only: bool,
}

impl Attempt {
    /// How a failure of this attempt is named.
    fn describe(self) -> &'static str {
        if self.software_only {
            "a software adapter"
        } else {
            "the preferred adapter"
        }
    }
}

/// The attempts `policy` makes, in order, each carrying `power`. Pure.
pub(super) fn attempts(policy: AdapterPolicy, power: PowerPreference) -> Vec<Attempt> {
    let preferred = Attempt {
        power_preference: power,
        software_only: false,
    };
    let software = Attempt {
        power_preference: power,
        software_only: true,
    };
    match policy {
        AdapterPolicy::Auto => vec![preferred, software],
        AdapterPolicy::Software => vec![software],
    }
}

/// Whether `next` can find anything the attempts before it did not:
/// not when it asks for a software adapter and an earlier attempt
/// already failed on one — `request_adapter` ranks software adapters
/// last and would hand the same one back. `failed_on` is the device
/// type of every adapter an earlier attempt got but could not open a
/// device on. Pure.
pub(super) fn worth_retrying(failed_on: &[DeviceType], next: Attempt) -> bool {
    !(next.software_only && failed_on.contains(&DeviceType::Cpu))
}

/// The first of `names` that contains `wanted`, ignoring case — the
/// meaning wgpu gives `WGPU_ADAPTER_NAME` — or a message naming what
/// there was to choose from. Pure.
pub(super) fn match_name(names: &[String], wanted: &str) -> Result<usize, String> {
    let needle = wanted.to_lowercase();
    names
        .iter()
        .position(|name| name.to_lowercase().contains(&needle))
        .ok_or_else(|| {
            if names.is_empty() {
                format!("WGPU_ADAPTER_NAME={wanted:?}, but no adapter qualifies")
            } else {
                format!(
                    "WGPU_ADAPTER_NAME={wanted:?} matches none of the adapters that qualify: {}",
                    names.join(", ")
                )
            }
        })
}

/// The instance every bootstrap creates its adapter through: wgpu's
/// defaults with its environment applied — `WGPU_BACKEND` among it —
/// and no display handle, as `Instance::default()` had.
pub(super) fn instance_descriptor() -> wgpu::InstanceDescriptor {
    wgpu::InstanceDescriptor::new_without_display_handle().with_env()
}

/// The power preference every attempt carries: `WGPU_POWER_PREF`, or
/// high performance when it is unset or unreadable.
fn power_preference() -> PowerPreference {
    PowerPreference::from_env().unwrap_or(PowerPreference::HighPerformance)
}

/// An adapter and the device opened on it, through the plan above:
/// `WGPU_ADAPTER_NAME` first when it is set, else each [`Attempt`] in
/// turn until one opens a device. `surface` is the window's, when the
/// renderer presents to one; the headless target has none. `Err`
/// carries every attempt's failure, for the bootstrap's startup panic.
pub(super) async fn acquire(
    instance: &Instance,
    surface: Option<&Surface<'static>>,
    policy: AdapterPolicy,
) -> Result<(Adapter, Device, Queue), String> {
    if let Ok(wanted) = std::env::var("WGPU_ADAPTER_NAME") {
        return by_name(instance, surface, policy, &wanted).await;
    }
    let power = power_preference();
    let mut failures: Vec<String> = Vec::new();
    let mut failed_on: Vec<DeviceType> = Vec::new();
    for attempt in attempts(policy, power) {
        if !worth_retrying(&failed_on, attempt) {
            continue;
        }
        let requested = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: attempt.power_preference,
                force_fallback_adapter: attempt.software_only,
                compatible_surface: surface,
                // Limit bucketing is a fingerprinting defense for
                // hosts that expose wgpu to untrusted content. This
                // app runs only its own code, so it keeps the
                // adapter's real limits, as before the option existed.
                apply_limit_buckets: false,
            })
            .await;
        let adapter = match requested {
            Ok(adapter) => adapter,
            Err(e) => {
                failures.push(format!("{}: {e}", attempt.describe()));
                continue;
            }
        };
        let info = adapter.get_info();
        match open_device(&adapter).await {
            Ok((device, queue)) => {
                log::info!(
                    "renderer: adapter {:?} ({:?}, {:?})",
                    info.name,
                    info.backend,
                    info.device_type
                );
                return Ok((adapter, device, queue));
            }
            Err(e) => {
                failures.push(format!("{} {:?}: {e}", attempt.describe(), info.name));
                failed_on.push(info.device_type);
            }
        }
    }
    Err(failures.join("; "))
}

/// The `WGPU_ADAPTER_NAME` branch of [`acquire`]: every adapter the
/// instance exposes that could present to `surface` (when there is
/// one) and that `policy` admits, the first whose name matches, and a
/// device on it.
async fn by_name(
    instance: &Instance,
    surface: Option<&Surface<'static>>,
    policy: AdapterPolicy,
    wanted: &str,
) -> Result<(Adapter, Device, Queue), String> {
    let mut adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
    adapters.retain(|adapter| {
        surface.is_none_or(|surface| adapter.is_surface_supported(surface))
            && (policy == AdapterPolicy::Auto || adapter.get_info().device_type == DeviceType::Cpu)
    });
    let names: Vec<String> = adapters.iter().map(|adapter| adapter.get_info().name).collect();
    let adapter = adapters.swap_remove(match_name(&names, wanted)?);
    let (device, queue) = open_device(&adapter).await.map_err(|e| {
        format!(
            "adapter {:?} could not open a device: {e}",
            adapter.get_info().name
        )
    })?;
    Ok((adapter, device, queue))
}

/// Whether this machine has an adapter that can open a device under
/// `policy`, through [`acquire`] itself — the plan the app runs, with
/// no surface, as the headless bootstrap asks. `Ok` names the adapter;
/// `Err` carries every attempt's failure. For the GPU-backed test
/// harness (`app::gpu_harness`), which asks once per process and skips
/// with a note on `Err`. The device opened here is dropped with the
/// answer: the probe says whether an app *could* be built, and the
/// harness then builds one through the same bootstrap the app uses.
#[cfg(test)]
pub(crate) async fn probe_adapter(policy: AdapterPolicy) -> Result<String, String> {
    let instance = Instance::new(instance_descriptor());
    let (adapter, _device, _queue) = acquire(&instance, None, policy).await?;
    Ok(adapter.get_info().name)
}

/// A device and its queue on `adapter`, with the limits every target
/// shares: the downlevel defaults, raised to what the adapter offers.
async fn open_device(adapter: &Adapter) -> Result<(Device, Queue), wgpu::RequestDeviceError> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            memory_hints: Default::default(),
            trace: Default::default(),
            experimental_features: Default::default(),
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Auto` asks for the preferred adapter first and a software one
    /// second; `Software` asks for the software one alone. Both carry
    /// the power preference they were given.
    #[test]
    fn test_the_plan_tries_the_preferred_adapter_before_a_software_one() {
        let auto = attempts(AdapterPolicy::Auto, PowerPreference::LowPower);
        assert_eq!(
            auto,
            vec![
                Attempt {
                    power_preference: PowerPreference::LowPower,
                    software_only: false
                },
                Attempt {
                    power_preference: PowerPreference::LowPower,
                    software_only: true
                },
            ]
        );
        assert_eq!(
            attempts(AdapterPolicy::Software, PowerPreference::HighPerformance),
            vec![Attempt {
                power_preference: PowerPreference::HighPerformance,
                software_only: true
            }]
        );
    }

    /// The software attempt runs after a hardware adapter failed to open
    /// a device, and is skipped after a software one did: it would get
    /// that adapter again. A non-software attempt is always worth making.
    #[test]
    fn test_a_software_retry_is_skipped_only_after_a_software_adapter_failed() {
        let [preferred, software] = attempts(AdapterPolicy::Auto, PowerPreference::HighPerformance)[..]
        else {
            panic!("two attempts");
        };
        assert!(
            worth_retrying(&[], software),
            "the first attempt never got an adapter"
        );
        assert!(worth_retrying(&[DeviceType::DiscreteGpu], software));
        assert!(worth_retrying(
            &[DeviceType::IntegratedGpu, DeviceType::Other],
            software
        ));
        assert!(
            !worth_retrying(&[DeviceType::Cpu], software),
            "the same adapter again"
        );
        assert!(worth_retrying(&[DeviceType::Cpu], preferred));
    }

    /// The first adapter whose name contains the wanted text, case
    /// aside — wgpu's own meaning — and otherwise a message that lists
    /// what there was.
    #[test]
    fn test_an_adapter_is_matched_by_name_as_wgpu_matches_it() {
        let names: Vec<String> = [
            "NVIDIA GeForce RTX 4070",
            "llvmpipe (LLVM 19.1.1, 256 bits)",
            "lavapipe",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(match_name(&names, "LLVMPIPE"), Ok(1));
        assert_eq!(match_name(&names, "pipe"), Ok(1), "the first match wins");
        assert_eq!(match_name(&names, "geforce"), Ok(0));
        let refused = match_name(&names, "radeon").expect_err("nothing matches");
        assert!(refused.contains("\"radeon\""), "{refused}");
        assert!(refused.contains("lavapipe"), "the choices are named: {refused}");
        assert!(match_name(&[], "x")
            .expect_err("nothing to match")
            .contains("no adapter qualifies"));
    }
}
