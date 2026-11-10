//! Desktop-only safe compute adapter. Core crates never depend on wgpu.
use eframe::wgpu::{self, util::DeviceExt};
use render::{
    Error, ExecutionHooks, Image, MemoryKind, Result,
    effects::{EffectCompositor, EffectInputs, EffectPass},
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, mpsc},
    time::Duration,
};

#[derive(Clone)]
struct ComputePass<'a> {
    entry_point: &'static str,
    horizontal: bool,
    values: Cow<'a, [f32]>,
    /// Reads the instance input image as well as the previous pass output.
    sample: bool,
}

pub struct GpuEffects {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: BTreeMap<&'static str, wgpu::ComputePipeline>,
}
impl GpuEffects {
    /// Offscreen effect tiles and background-blur previews share this bounded
    /// engine cache and the existing GPU adapter; no window is required.
    pub fn thumbnails(
        self: &std::sync::Arc<Self>,
        budget: engine::ByteBudget,
    ) -> engine::effect_thumbnails::EffectThumbnails {
        engine::effect_thumbnails::EffectThumbnails::new(self.clone(), budget)
    }

    pub async fn new(device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let entries: Vec<_> = (0..7)
            .map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: if binding == 3 {
                        wgpu::BufferBindingType::Uniform
                    } else {
                        wgpu::BufferBindingType::Storage {
                            read_only: binding != 1,
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effects"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("effects"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::include_wgsl!("effects.wgsl"));
        let pipelines = ["gaussian", "blend", "grade", "chroma_key", "mirror"]
            .into_iter()
            .map(|entry| {
                (
                    entry,
                    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some(entry),
                        layout: Some(&pipeline_layout),
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                )
            })
            .collect();
        if scope.pop().await.is_some() {
            return Err(Error::Unsupported("GPU effect pipeline"));
        }
        Ok(Self {
            device,
            queue,
            layout,
            pipelines,
        })
    }
    fn buffer(&self, bytes: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("effect data"),
                contents: bytes,
                usage,
            })
    }
    fn apply_inner<'a>(
        &self,
        item: &Image,
        passes: impl ExactSizeIterator<Item = ComputePass<'a>> + Clone,
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        let bytes = Image::bytes(item.width(), item.height())?;
        let limits = self.device.limits();
        if bytes > limits.max_storage_buffer_binding_size
            || bytes > limits.max_buffer_size
            || item.width().div_ceil(8) > limits.max_compute_workgroups_per_dimension
            || item.height().div_ceil(8) > limits.max_compute_workgroups_per_dimension
        {
            return Err(Error::Unsupported("GPU effect dimensions"));
        }
        inputs.validate(item)?;
        for pass in passes.clone() {
            if !self.pipelines.contains_key(pass.entry_point) {
                return Err(Error::Unsupported("GPU shader entry"));
            }
        }
        // two working buffers, readback, below, coverage, one upload vector;
        // plus per-pass uniforms/weights and conservative native buffer padding.
        let pass_bytes = (passes.len() as u64)
            .checked_mul(1024)
            .ok_or(Error::Invalid("GPU pass bytes"))?;
        let lease = Arc::new(hooks.reserve(MemoryKind::Compositor, bytes * 7 + pass_bytes)?);
        let upload = |image: &Image| -> Result<wgpu::Buffer> {
            let mut data = Vec::new();
            data.try_reserve_exact(bytes as usize)?;
            for pixel in image.pixels() {
                for v in pixel {
                    data.extend_from_slice(&v.to_le_bytes());
                }
            }
            Ok(self.buffer(
                &data,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            ))
        };
        let mut a = upload(item)?;
        // A sample pass also reads the image its effect instance started with;
        // other runs leave binding 6 on the current image instead of uploading.
        let instance = if passes.clone().any(|pass| pass.sample) {
            Some(upload(inputs.input)?)
        } else {
            None
        };
        let mut b = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect output"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let below = upload(inputs.below)?;
        let coverage = upload(inputs.coverage.unwrap_or(item))?;
        let download = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        for pass in passes {
            if hooks.cancelled() {
                return Err(Error::Cancelled);
            }
            let constants: Vec<u8> = [
                item.width(),
                item.height(),
                u32::from(pass.horizontal),
                pass.values.len() as u32,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
            let params = self.buffer(&constants, wgpu::BufferUsages::UNIFORM);
            let weights: Vec<u8> = pass.values.iter().flat_map(|w| w.to_le_bytes()).collect();
            let weights = self.buffer(&weights, wgpu::BufferUsages::STORAGE);
            let entries: Vec<_> = [
                &a,
                &b,
                &weights,
                &params,
                &coverage,
                &below,
                instance.as_ref().unwrap_or(&a),
            ]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
            let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("effect pass"),
                layout: &self.layout,
                entries: &entries,
            });
            let mut compute = encoder.begin_compute_pass(&Default::default());
            compute.set_pipeline(
                self.pipelines
                    .get(pass.entry_point)
                    .ok_or(Error::Unsupported("GPU shader entry"))?,
            );
            compute.set_bind_group(0, &group, &[]);
            compute.dispatch_workgroups(item.width().div_ceil(8), item.height().div_ceil(8), 1);
            drop(compute);
            std::mem::swap(&mut a, &mut b);
        }
        encoder.copy_buffer_to_buffer(&a, 0, &download, 0, bytes);
        let index = self.queue.submit([encoder.finish()]);
        // If a device wait times out, submitted native work still owns its bytes.
        let submitted_lease = lease.clone();
        self.queue
            .on_submitted_work_done(move || drop(submitted_lease));
        let slice = download.slice(..);
        let (tx, rx) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(index),
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(|_| Error::Unsupported("GPU effect wait"))?;
        rx.recv_timeout(Duration::from_secs(1))
            .map_err(|_| Error::Unsupported("GPU readback timeout"))?
            .map_err(|_| Error::Unsupported("GPU readback"))?;
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        let data = slice
            .get_mapped_range()
            .map_err(|_| Error::Unsupported("GPU mapped range"))?;
        let pixels = data.as_chunks::<16>().0.iter().map(|p| {
            let mut pixel = [0.0; 4];
            for (v, bytes) in pixel.iter_mut().zip(p.as_chunks::<4>().0) {
                *v = f32::from_le_bytes(*bytes);
            }
            pixel
        });
        let result = Image::from_working_pixels(item.width(), item.height(), pixels, hooks);
        drop(data);
        download.unmap();
        result
    }

    fn execute<'a>(
        &self,
        item: &Image,
        passes: impl ExactSizeIterator<Item = ComputePass<'a>> + Clone,
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        // wgpu std error scopes are thread-local; buffers/submissions are call-local.
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self.device.push_error_scope(wgpu::ErrorFilter::Internal);
        let result = self.apply_inner(item, passes, inputs, hooks);
        let failed = pollster::block_on(internal.pop());
        let oom = pollster::block_on(allocation.pop());
        let invalid = pollster::block_on(validation.pop());
        if failed.is_some() || oom.is_some() || invalid.is_some() {
            return Err(Error::Unsupported("GPU effect execution"));
        }
        result
    }
}
impl EffectCompositor for GpuEffects {
    fn supports(&self, entry_point: &str) -> bool {
        self.pipelines.contains_key(entry_point)
    }
    fn composite(
        &self,
        below: &mut Image,
        item: &Image,
        opacity: f32,
        mode: project::BlendMode,
        hooks: &dyn ExecutionHooks,
    ) -> Result<()> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(Error::Invalid("GPU blend opacity"));
        }
        let values = [opacity, mode as u32 as f32];
        let pass = ComputePass {
            entry_point: "blend",
            horizontal: false,
            values: Cow::Borrowed(&values),
            sample: false,
        };
        let result = self.execute(
            item,
            std::iter::once(pass),
            EffectInputs {
                input: item,
                coverage: None,
                below,
            },
            hooks,
        )?;
        *below = result;
        Ok(())
    }
    fn apply(
        &self,
        item: &Image,
        passes: &[EffectPass],
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        for pass in passes {
            render::effects::validate_pass(pass, item.width(), item.height())?;
        }
        self.execute(
            item,
            passes.iter().map(|pass| ComputePass {
                entry_point: pass.entry_point(),
                horizontal: pass.horizontal(),
                values: pass.values(),
                sample: matches!(pass, EffectPass::Sample { .. }),
            }),
            inputs,
            hooks,
        )
    }
}
