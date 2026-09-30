//! GPU capability probe (`--probe-gpu`).
//!
//! `runtime_self_check` validates assets, the tile seed and the local store,
//! but it never creates a graphics context — so CI passes on machines with
//! no display, and nothing in the build currently proves the app renders.
//! This does: instance, adapter, device, one off-screen pass, timed.
//!
//! It reports `wgpu::DeviceType` rather than sniffing the adapter name,
//! because `device_type` is a typed field and `Cpu` is a real variant — that
//! is the signal a quality tier branches on, where `"llvmpipe".contains(..)`
//! is a guess. A software rasteriser is not a broken machine, so the budget
//! breach is reported loudly but only a *missing* adapter is fatal.
//!
//! Every wgpu call here is deliberately a hand-built descriptor rather than a
//! `::default()`: the defaults are what an app picks, and this is a check on
//! the stack, so each value is visible in the source.

/// Reported, not silently tolerated: a 256px clear past this means software
/// rendering, which is a quality decision rather than a failure.
pub const BUDGET: std::time::Duration = std::time::Duration::from_millis(100);

/// Never let a wedged GPU hang a build.
const POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Probe the graphics stack and report what the console would render on.
///
/// Returns `Err` when there is no adapter at all, and also when the frame
/// blows `BUDGET` — so a caller can fail a build on either, while a caller
/// that would rather read the report can ignore the exit code. CI on a
/// headless container should do the latter: software rendering there is
/// expected, not a regression.
pub fn probe() -> Result<(), String> {
    // egui-wgpu re-exports the exact wgpu it renders with, so this cannot
    // drift from the app's version and costs no dependency.
    use eframe::egui_wgpu::wgpu;

    // `request_device` and `enumerate_adapters` are both async in wgpu 30,
    // and `main` is sync with no runtime up yet.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| format!("probe runtime: {e}"))?;

    let started = std::time::Instant::now();
    let (device_type, device_ready, frame) = rt.block_on(async {
        // Defaults cover all backends and pick FXC for the DX12 shader
        // compiler, which is not what the app asks for but does not matter:
        // this probe never compiles a shader.
        let instance = wgpu::Instance::default();

        let mut adapters = instance.enumerate_adapters(wgpu::Backends::all()).await;
        if adapters.is_empty() {
            return Err(
                "no wgpu adapter: this machine has no GPU driver and no software rasteriser, \
                 so the console cannot render"
                    .to_string(),
            );
        }

        // Best first, so the report describes what the app would actually
        // pick. This mirrors egui-wgpu's `PowerPreference::default` bias
        // rather than inventing a policy of its own.
        adapters.sort_by_key(|a| match a.get_info().device_type {
            wgpu::DeviceType::DiscreteGpu => 0,
            wgpu::DeviceType::IntegratedGpu => 1,
            wgpu::DeviceType::VirtualGpu => 2,
            wgpu::DeviceType::Other => 3,
            wgpu::DeviceType::Cpu => 4,
        });
        for adapter in &adapters {
            let info = adapter.get_info();
            println!(
                "adapter: {:?} / {:?} — {}",
                info.backend, info.device_type, info.name
            );
        }

        let adapter = adapters.swap_remove(0);
        let device_type = adapter.get_info().device_type;

        // On a software rasteriser the stock limits may be unsatisfiable,
        // which is exactly the case a quality tier has to survive — so the
        // probe asks for the floor rather than the ceiling there.
        let limits = if device_type == wgpu::DeviceType::Cpu {
            wgpu::Limits::downlevel_defaults()
        } else {
            wgpu::Limits::default()
        };
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("tfg probe"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                ..Default::default()
            })
            .await
            .map_err(|e| format!("device request failed on {device_type:?}: {e}"))?;
        let device_ready = started.elapsed();

        // ---- timed from here ----
        // Driver init and pipeline-cache warm-up are one-time costs that
        // land in `device_ready` and say nothing about render cost. An
        // earlier version timed from process start and read 2.1 s on a
        // discrete card: that was Vulkan initialisation, not a frame.
        let frame_start = std::time::Instant::now();

        // One real pass, off-screen: no surface is involved, so this works
        // in a headless container and still measures the pipeline.
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("probe target"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("probe") });
        {
            let pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("probe"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            // Console Night, so the target holds the app's
                            // own darkest chrome value and the clear is a
                            // real write rather than a no-op.
                            r: 0.059,
                            g: 0.090,
                            b: 0.165,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // No `end()` in wgpu 30 — the pass is finalised on drop, and the
            // borrow of `enc` ends with the block.
            drop(pass);
        }
        queue.submit([enc.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(POLL_TIMEOUT),
            })
            .map_err(|e| format!("device poll: {e}"))?;
        let frame = frame_start.elapsed();
        Ok::<_, String>((device_type, device_ready, frame))
    })?;

    println!(
        "probe: {device_type:?} — device ready in {device_ready:?}, \
         first frame in {frame:?} (budget {BUDGET:?})"
    );

    if frame > BUDGET {
        // Name the real cause. An earlier version reported "software
        // rasteriser" here unconditionally, which contradicted its own
        // `device_type` — a slow real GPU and a CPU rasteriser are
        // different diagnoses with different fixes.
        let cause = if device_type == wgpu::DeviceType::Cpu {
            "this is a software rasteriser; treat the console as LOW quality"
        } else {
            "a real GPU was too slow for one 256px clear — not a rasteriser \
             problem, and worth investigating before shipping"
        };
        return Err(format!("first frame took {frame:?}, over budget: {cause}"));
    }
    if device_type == wgpu::DeviceType::Cpu {
        println!("note: software rasteriser — LOW quality regardless of timing");
    }
    Ok(())
}
