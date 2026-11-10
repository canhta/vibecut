//! Desktop-only safe compute adapter. Core crates never depend on wgpu.
use eframe::wgpu;
use render::{
    Error, ExecutionHooks, Image, MemoryKind, Result,
    effects::{EffectCompositor, EffectInputs, EffectPass},
};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
struct ComputePass<'a> {
    entry_point: &'static str,
    horizontal: bool,
    values: Cow<'a, [f32]>,
}

#[derive(Clone, Copy, PartialEq)]
enum ReadImage {
    Instance,
    Below,
}

fn read_image(entry: &str) -> Result<Option<ReadImage>> {
    match entry {
        "gaussian" | "grade" | "chroma_key" => Ok(None),
        "mirror" => Ok(Some(ReadImage::Instance)),
        "blend" => Ok(Some(ReadImage::Below)),
        _ => Err(Error::Unsupported("GPU shader inputs")),
    }
}

pub struct GpuEffects {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    pipelines: BTreeMap<&'static str, wgpu::ComputePipeline>,
    busy: Arc<AtomicBool>,
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
            busy: Arc::new(AtomicBool::new(false)),
        })
    }
    fn open<'a>(&'a self, hooks: &'a dyn ExecutionHooks) -> Result<FrameEffects<'a>> {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        self.device
            .poll(wgpu::PollType::Poll)
            .map_err(|_| Error::Unsupported("GPU retirement"))?;
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Unsupported("GPU frame busy"))?;
        let mut permit = Permit {
            busy: self.busy.clone(),
            lease: None,
        };
        let records = std::mem::size_of::<FrameEffects<'_>>()
            + std::mem::size_of::<Permit>()
            + std::mem::size_of::<Flight>()
            + std::mem::size_of::<Scratch>()
            + std::mem::size_of::<Core>()
            + std::mem::size_of::<Aux>()
            + std::mem::size_of::<Metadata>();
        if records > 4096 {
            return Err(Error::Unsupported("GPU scope records"));
        }
        permit.lease = Some(hooks.reserve(MemoryKind::Compositor, records as u64)?);
        Ok(FrameEffects {
            gpu: self,
            hooks,
            state: Mutex::new(State::default()),
            permit: Arc::new(permit),
            #[cfg(test)]
            boundary: None,
        })
    }
}

struct Permit {
    busy: Arc<AtomicBool>,
    lease: Option<Box<dyn render::Reservation>>,
}
impl Drop for Permit {
    fn drop(&mut self) {
        drop(self.lease.take());
        self.busy.store(false, Ordering::Release);
    }
}
// Resource handles precede their owned leases; scope/submission permits drop last.
struct Core {
    a: wgpu::Buffer,
    b: wgpu::Buffer,
    download: wgpu::Buffer,
    upload: wgpu::Buffer,
    _lease: Box<dyn render::Reservation>,
}
struct Aux {
    data: wgpu::Buffer,
    upload: wgpu::Buffer,
    _lease: Box<dyn render::Reservation>,
}
struct Metadata {
    slots: Box<[(wgpu::Buffer, wgpu::Buffer)]>,
    upload: wgpu::Buffer,
    _lease: Box<dyn render::Reservation>,
}
#[derive(Clone, Default)]
struct Scratch {
    core: Option<Arc<Core>>,
    aux: Option<Arc<Aux>>,
    meta: Option<Arc<Metadata>>,
}
#[derive(Default)]
struct State {
    geometry: Option<(u32, u32)>,
    scratch: Scratch,
    running: bool,
    retired: bool,
}
struct FrameEffects<'a> {
    gpu: &'a GpuEffects,
    hooks: &'a dyn ExecutionHooks,
    state: Mutex<State>,
    permit: Arc<Permit>,
    #[cfg(test)]
    boundary: Option<Arc<tests::Boundary>>,
}
struct Flight {
    _scratch: Scratch,
    done: AtomicBool,
    _permit: Arc<Permit>,
}
// Own conservative custody BEFORE calling submit: unwind between submit and callback
// registration must also retain native resources and their charges through completion.
struct Submission<'a> {
    queue: &'a wgpu::Queue,
    flight: Arc<Flight>,
    possible: bool,
    registered: bool,
}
impl Submission<'_> {
    fn register_with(&mut self, register: impl FnOnce(Box<dyn FnOnce() + Send>)) {
        let flight = self.flight.clone();
        register(Box::new(move || {
            flight.done.store(true, Ordering::Release);
        }));
        self.registered = true;
    }
    fn register(&mut self) {
        let queue = self.queue;
        self.register_with(|done| queue.on_submitted_work_done(done));
    }
}
impl Drop for Submission<'_> {
    fn drop(&mut self) {
        if self.possible && !self.registered {
            self.register();
        }
    }
}
struct Mappings<'a> {
    scratch: &'a Scratch,
    ready: bool,
}
impl Drop for Mappings<'_> {
    fn drop(&mut self) {
        if let Some(core) = &self.scratch.core {
            core.download.unmap();
            if !self.ready {
                core.upload.unmap();
            }
        }
        if !self.ready {
            if let Some(aux) = &self.scratch.aux {
                aux.upload.unmap();
            }
            if let Some(meta) = &self.scratch.meta {
                meta.upload.unmap();
            }
        }
    }
}
impl FrameEffects<'_> {
    fn hooks(&self, hooks: &dyn ExecutionHooks) -> Result<()> {
        if !std::ptr::eq(self.hooks, hooks) {
            return Err(Error::Invalid("GPU scope hooks"));
        }
        if self.hooks.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn buffer(&self, size: u64, usage: wgpu::BufferUsages, mapped: bool) -> wgpu::Buffer {
        self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect scratch"),
            size,
            usage,
            mapped_at_creation: mapped,
        })
    }
    fn pack(&self, image: &Image, upload: &wgpu::Buffer) -> Result<()> {
        let mut data = upload
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|_| Error::Unsupported("GPU upload mapping"))?;
        for (index, pixel) in image.pixels().iter().enumerate() {
            for (channel, value) in pixel.iter().enumerate() {
                let offset = index * 16 + channel * 4;
                data.slice(offset..offset + 4)
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        drop(data);
        upload.unmap();
        Ok(())
    }
    fn execute<'p>(
        &self,
        item: &Image,
        passes: impl ExactSizeIterator<Item = ComputePass<'p>> + Clone,
        inputs: EffectInputs<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Image> {
        self.hooks(hooks)?;
        inputs.validate(item)?;
        let bytes = Image::bytes(item.width(), item.height())?;
        let limits = self.gpu.device.limits();
        if bytes > limits.max_storage_buffer_binding_size
            || bytes > limits.max_buffer_size
            || item.width().div_ceil(8) > limits.max_compute_workgroups_per_dimension
            || item.height().div_ceil(8) > limits.max_compute_workgroups_per_dimension
        {
            return Err(Error::Unsupported("GPU effect dimensions"));
        }
        if passes.len() > 512 {
            return Err(Error::Unsupported("GPU pass capacity"));
        }
        let mut read = None;
        for pass in passes.clone() {
            if !self.gpu.supports(pass.entry_point) || pass.values.len() > 49 {
                return Err(Error::Unsupported("GPU shader entry"));
            }
            if let Some(next) = read_image(pass.entry_point)? {
                if read.is_some_and(|previous| previous != next) {
                    return Err(Error::Unsupported("GPU auxiliary inputs"));
                }
                read = Some(next);
            }
        }
        let mut scratch = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::Invalid("GPU scope state"))?;
            if state.running || state.retired {
                return Err(Error::Unsupported("GPU scope unavailable"));
            }
            let geometry = (item.width(), item.height());
            if state.geometry.is_some_and(|first| first != geometry) {
                return Err(Error::Unsupported("GPU scope geometry"));
            }
            state.geometry = Some(geometry);
            state.running = true;
            std::mem::take(&mut state.scratch)
        };
        let validation = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Validation);
        let allocation = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = self
            .gpu
            .device
            .push_error_scope(wgpu::ErrorFilter::Internal);
        let mut submitted = false;
        let result = self.run(
            item,
            passes,
            inputs,
            read,
            bytes,
            &mut scratch,
            &mut submitted,
        );
        let failed = pollster::block_on(internal.pop()).is_some();
        #[cfg(test)]
        let failed = failed
            || self
                .boundary
                .as_ref()
                .is_some_and(|b| matches!(b.fault, tests::Fault::Internal | tests::Fault::Cancel));
        let oom = pollster::block_on(allocation.pop());
        let invalid = pollster::block_on(validation.pop());
        let result =
            if self.hooks.cancelled() && matches!(&result, Ok(_) | Err(Error::Unsupported(_))) {
                Err(Error::Cancelled)
            } else {
                result
            };
        let result = if (failed || oom.is_some() || invalid.is_some())
            && matches!(&result, Ok(_) | Err(Error::Unsupported(_)))
        {
            Err(Error::Unsupported("GPU effect execution"))
        } else {
            result
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::Invalid("GPU scope state"))?;
        state.running = false;
        if result.is_ok() {
            state.scratch = scratch;
        } else {
            state.retired = submitted;
        }
        result
    }
    #[allow(clippy::too_many_arguments)]
    fn run<'p>(
        &self,
        item: &Image,
        passes: impl ExactSizeIterator<Item = ComputePass<'p>> + Clone,
        inputs: EffectInputs<'_>,
        read: Option<ReadImage>,
        bytes: u64,
        scratch: &mut Scratch,
        submitted: &mut bool,
    ) -> Result<Image> {
        use wgpu::BufferUsages as U;
        let core_new = scratch.core.is_none();
        if core_new {
            let lease = self.hooks.reserve(
                MemoryKind::Compositor,
                bytes
                    .checked_mul(4)
                    .ok_or(Error::Invalid("GPU working bytes"))?,
            )?;
            scratch.core = Some(Arc::new(Core {
                a: self.buffer(bytes, U::STORAGE | U::COPY_SRC | U::COPY_DST, false),
                b: self.buffer(bytes, U::STORAGE | U::COPY_SRC | U::COPY_DST, false),
                download: self.buffer(bytes, U::COPY_DST | U::MAP_READ, false),
                upload: self.buffer(bytes, U::COPY_SRC | U::MAP_WRITE, true),
                _lease: lease,
            }));
        }
        if read.is_none() {
            scratch.aux = None;
        }
        let aux_new = read.is_some() && scratch.aux.is_none();
        if aux_new {
            let lease = self.hooks.reserve(
                MemoryKind::Compositor,
                bytes
                    .checked_mul(2)
                    .ok_or(Error::Invalid("GPU auxiliary bytes"))?,
            )?;
            scratch.aux = Some(Arc::new(Aux {
                data: self.buffer(bytes, U::STORAGE | U::COPY_DST, false),
                upload: self.buffer(bytes, U::MAP_WRITE | U::COPY_SRC, true),
                _lease: lease,
            }));
        }
        let count = passes.len().max(1);
        if count as u64 * 212 > self.gpu.device.limits().max_buffer_size {
            return Err(Error::Unsupported("GPU metadata dimensions"));
        }
        let meta_new = scratch
            .meta
            .as_ref()
            .is_none_or(|meta| meta.slots.len() < count);
        if meta_new {
            scratch.meta = None;
            // 424 native bytes/slot plus actual host handles and transient flattened
            // values, rounded conservatively within the documented 1024/pass bound.
            let host = std::mem::size_of::<(wgpu::Buffer, wgpu::Buffer)>() as u64 + 49 * 4;
            if 424 + host > 1024 {
                return Err(Error::Unsupported("GPU metadata capacity"));
            }
            let lease = self
                .hooks
                .reserve(MemoryKind::Compositor, count as u64 * 1024)?;
            let mut slots = Vec::new();
            slots.try_reserve_exact(count)?;
            for _ in 0..count {
                slots.push((
                    self.buffer(16, U::UNIFORM | U::COPY_DST, false),
                    self.buffer(196, U::STORAGE | U::COPY_DST, false),
                ));
            }
            scratch.meta = Some(Arc::new(Metadata {
                slots: slots.into_boxed_slice(),
                upload: self.buffer(count as u64 * 212, U::MAP_WRITE | U::COPY_SRC, true),
                _lease: lease,
            }));
        }
        let core = scratch.core.as_ref().ok_or(Error::Invalid("GPU core"))?;
        let meta = scratch
            .meta
            .as_ref()
            .ok_or(Error::Invalid("GPU metadata"))?;
        let mut mappings = Mappings {
            scratch,
            ready: false,
        };
        self.pack(item, &core.upload)?;
        if let Some(aux) = &scratch.aux {
            self.pack(
                if read == Some(ReadImage::Instance) {
                    inputs.input
                } else {
                    inputs.below
                },
                &aux.upload,
            )?;
        }
        let mut data = meta
            .upload
            .slice(..)
            .get_mapped_range_mut()
            .map_err(|_| Error::Unsupported("GPU metadata mapping"))?;
        for (index, pass) in passes.clone().enumerate() {
            let constants = [
                item.width(),
                item.height(),
                u32::from(pass.horizontal),
                pass.values.len() as u32,
            ];
            for (channel, value) in constants.into_iter().enumerate() {
                let offset = index * 212 + channel * 4;
                data.slice(offset..offset + 4)
                    .copy_from_slice(&value.to_le_bytes());
            }
            for (channel, value) in pass.values.iter().enumerate() {
                let offset = index * 212 + 16 + channel * 4;
                data.slice(offset..offset + 4)
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        drop(data);
        meta.upload.unmap();
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(&core.upload, 0, &core.a, 0, bytes);
        if let Some(aux) = &scratch.aux {
            encoder.copy_buffer_to_buffer(&aux.upload, 0, &aux.data, 0, bytes);
        }
        let mut a = &core.a;
        let mut b = &core.b;
        for (index, pass) in passes.enumerate() {
            if self.hooks.cancelled() {
                return Err(Error::Cancelled);
            }
            let (params, weights) = meta
                .slots
                .get(index)
                .ok_or(Error::Invalid("GPU pass slot"))?;
            encoder.copy_buffer_to_buffer(&meta.upload, index as u64 * 212, params, 0, 16);
            encoder.copy_buffer_to_buffer(&meta.upload, index as u64 * 212 + 16, weights, 0, 196);
            let read = read_image(pass.entry_point)?;
            let auxiliary = scratch.aux.as_ref().map_or(a, |aux| &aux.data);
            let entries = [
                a,
                b,
                weights,
                params,
                a,
                if read == Some(ReadImage::Below) {
                    auxiliary
                } else {
                    a
                },
                if read == Some(ReadImage::Instance) {
                    auxiliary
                } else {
                    a
                },
            ]
            .into_iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect::<Vec<_>>();
            let group = self
                .gpu
                .device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("effect pass"),
                    layout: &self.gpu.layout,
                    entries: &entries,
                });
            let mut compute = encoder.begin_compute_pass(&Default::default());
            compute.set_pipeline(
                self.gpu
                    .pipelines
                    .get(pass.entry_point)
                    .ok_or(Error::Unsupported("GPU shader entry"))?,
            );
            compute.set_bind_group(0, &group, &[]);
            compute.dispatch_workgroups(item.width().div_ceil(8), item.height().div_ceil(8), 1);
            drop(compute);
            std::mem::swap(&mut a, &mut b);
        }
        encoder.copy_buffer_to_buffer(a, 0, &core.download, 0, bytes);
        let flight = Arc::new(Flight {
            _scratch: scratch.clone(),
            done: AtomicBool::new(false),
            _permit: self.permit.clone(),
        });
        let mut submission = Submission {
            queue: &self.gpu.queue,
            flight: flight.clone(),
            possible: false,
            registered: false,
        };
        *submitted = true;
        submission.possible = true;
        self.gpu.queue.submit([encoder.finish()]);
        #[cfg(test)]
        if let Some(boundary) = &self.boundary {
            boundary.after_submit();
        }
        let (tx, rx) = mpsc::sync_channel(4);
        let buffers = [
            Some((&core.download, wgpu::MapMode::Read)),
            Some((&core.upload, wgpu::MapMode::Write)),
            scratch
                .aux
                .as_ref()
                .map(|aux| (&aux.upload, wgpu::MapMode::Write)),
            Some((&meta.upload, wgpu::MapMode::Write)),
        ];
        let mut maps = 0;
        for (buffer, mode) in buffers.into_iter().flatten() {
            maps += 1;
            let tx = tx.clone();
            #[cfg(test)]
            let fail = self
                .boundary
                .as_ref()
                .is_some_and(|b| b.fault == tests::Fault::Map);
            buffer.slice(..).map_async(mode, move |result| {
                #[cfg(test)]
                let result = if fail {
                    Err(wgpu::BufferAsyncError)
                } else {
                    result
                };
                let _ = tx.try_send(result);
            });
        }
        #[cfg(test)]
        if let Some(boundary) = &self.boundary {
            submission.register_with(|done| boundary.hold(done));
            if boundary.fault == tests::Fault::Timeout {
                return Err(Error::Unsupported("GPU effect wait"));
            }
        } else {
            submission.register();
        }
        #[cfg(not(test))]
        submission.register();
        tracing::debug!(
            image_bytes = bytes,
            core_new,
            aux_new,
            meta_new,
            image_uploads = if read.is_some() { 2 } else { 1 },
            "GPU scratch submission"
        );
        self.gpu
            .device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(|error| match error {
                wgpu::PollError::Timeout => Error::Unsupported("GPU effect wait"),
                wgpu::PollError::WrongSubmissionIndex(..) => Error::Invalid("GPU submission index"),
            })?;
        let deadline = Instant::now() + Duration::from_secs(1);
        for _ in 0..maps {
            rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| Error::Unsupported("GPU mapping timeout"))?
                .map_err(|_| Error::Unsupported("GPU mapping"))?;
        }
        #[cfg(test)]
        if let Some(boundary) = &self.boundary {
            if boundary.fault == tests::Fault::Cancel {
                boundary.cancel.store(true, Ordering::Release);
            }
        }
        if self.hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        if !flight.done.load(Ordering::Acquire) {
            return Err(Error::Unsupported("GPU completion receipt"));
        }
        let data = core
            .download
            .slice(..)
            .get_mapped_range()
            .map_err(|_| Error::Unsupported("GPU mapped range"))?;
        let pixels = data.as_chunks::<16>().0.iter().map(|p| {
            let mut pixel = [0.0; 4];
            for (v, bytes) in pixel.iter_mut().zip(p.as_chunks::<4>().0) {
                *v = f32::from_le_bytes(*bytes);
            }
            pixel
        });
        let result = Image::from_working_pixels(item.width(), item.height(), pixels, self.hooks);
        drop(data);
        mappings.ready = result.is_ok();
        result
    }
}

impl EffectCompositor for GpuEffects {
    fn frame<'a>(
        &'a self,
        hooks: &'a dyn ExecutionHooks,
    ) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
        Ok(Some(Box::new(self.open(hooks)?)))
    }
    fn release_idle(&self) -> Result<()> {
        Ok(())
    }
    fn supports(&self, entry: &str) -> bool {
        self.pipelines.contains_key(entry) && read_image(entry).is_ok()
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
        inputs.validate(item)?;
        for pass in passes {
            render::effects::validate_pass(pass, item.width(), item.height())?;
        }
        self.open(hooks)?.apply(item, passes, inputs, hooks)
    }
    fn composite(
        &self,
        below: &mut Image,
        item: &Image,
        opacity: f32,
        mode: project::BlendMode,
        hooks: &dyn ExecutionHooks,
    ) -> Result<()> {
        if hooks.cancelled() {
            return Err(Error::Cancelled);
        }
        EffectInputs {
            input: item,
            below,
            coverage: None,
        }
        .validate(item)?;
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(Error::Invalid("GPU blend opacity"));
        }
        self.open(hooks)?
            .composite(below, item, opacity, mode, hooks)
    }
}
impl EffectCompositor for FrameEffects<'_> {
    fn frame<'a>(
        &'a self,
        _: &'a dyn ExecutionHooks,
    ) -> Result<Option<Box<dyn EffectCompositor + 'a>>> {
        Err(Error::Invalid("nested GPU frame"))
    }
    fn release_idle(&self) -> Result<()> {
        let scratch = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| Error::Invalid("GPU scope state"))?;
            if state.running {
                return Err(Error::Unsupported("GPU scope busy"));
            }
            std::mem::take(&mut state.scratch)
        };
        drop(scratch);
        Ok(())
    }
    fn supports(&self, entry_point: &str) -> bool {
        self.gpu.supports(entry_point)
    }
    fn composite(
        &self,
        below: &mut Image,
        item: &Image,
        opacity: f32,
        mode: project::BlendMode,
        hooks: &dyn ExecutionHooks,
    ) -> Result<()> {
        self.hooks(hooks)?;
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(Error::Invalid("GPU blend opacity"));
        }
        let values = [opacity, mode as u32 as f32];
        let pass = ComputePass {
            entry_point: "blend",
            horizontal: false,
            values: Cow::Borrowed(&values),
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
        self.hooks(hooks)?;
        for pass in passes {
            render::effects::validate_pass(pass, item.width(), item.height())?;
        }
        self.execute(
            item,
            passes.iter().map(|pass| ComputePass {
                entry_point: pass.entry_point(),
                horizontal: pass.horizontal(),
                values: pass.values(),
            }),
            inputs,
            hooks,
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(super) enum Fault {
        Timeout,
        Cancel,
        Map,
        Internal,
        Unwind,
    }
    pub(super) struct Boundary {
        pub fault: Fault,
        pub cancel: AtomicBool,
        held: Mutex<Option<Box<dyn FnOnce() + Send>>>,
        budget: engine::ByteBudget,
    }
    impl Boundary {
        pub fn after_submit(&self) {
            if self.fault == Fault::Unwind {
                panic!("injected submit-to-registration unwind");
            }
        }
        pub fn hold(&self, done: Box<dyn FnOnce() + Send>) {
            *self.held.lock().unwrap() = Some(done);
        }
    }
    impl ExecutionHooks for Boundary {
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::Acquire)
        }
        fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn render::Reservation>> {
            engine::ByteBudgetHooks::new(&self.budget, None).reserve(kind, bytes)
        }
    }
    #[test]
    fn submitted_resources_survive_native_failures_scope_drop_and_registration_unwind() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default()));
        if !adapter
            .as_ref()
            .is_ok_and(|a| a.get_info().device_type != wgpu::DeviceType::Cpu)
        {
            assert!(std::env::var_os("VIBECUT_REQUIRE_ORACLES").is_none());
            eprintln!("SKIP: no native GPU for custody");
            return;
        }
        let adapter = adapter.unwrap();
        eprintln!("custody adapter={:?}", adapter.get_info());
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let gpu = pollster::block_on(GpuEffects::new(device, queue)).unwrap();
        for (fault, context) in [
            (Fault::Timeout, "GPU effect wait"),
            (Fault::Cancel, ""),
            (Fault::Map, "GPU mapping"),
            (Fault::Internal, "GPU effect execution"),
            (Fault::Unwind, ""),
        ] {
            let boundary = Arc::new(Boundary {
                fault,
                cancel: AtomicBool::new(false),
                held: Mutex::new(None),
                budget: engine::ByteBudget::new(1 << 20).unwrap(),
            });
            let hooks: &dyn ExecutionHooks = boundary.as_ref();
            let image = Image::new(17, 9, [0.2; 4], hooks).unwrap();
            let mut below = Image::new(17, 9, [0.1; 4], hooks).unwrap();
            let original = below.pixels().to_vec();
            let mut scope = gpu.open(hooks).unwrap();
            scope.boundary = Some(boundary.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                scope.composite(&mut below, &image, 0.7, project::BlendMode::Normal, hooks)
            }));
            match fault {
                Fault::Unwind => assert!(result.is_err()),
                Fault::Cancel => assert!(matches!(result.unwrap(), Err(Error::Cancelled))),
                _ => assert!(
                    matches!(result.unwrap(), Err(Error::Unsupported(error)) if error == context)
                ),
            }
            assert_eq!(below.pixels(), original, "atomic backdrop {fault:?}");
            drop(scope);
            drop(image);
            drop(below);
            let charged = boundary.budget.snapshot().unwrap().used;
            assert!(
                charged >= 6 * Image::bytes(17, 9).unwrap(),
                "custody {fault:?}"
            );
            if fault != Fault::Unwind {
                boundary.cancel.store(false, Ordering::Release);
                assert!(matches!(
                    gpu.frame(hooks),
                    Err(Error::Unsupported("GPU frame busy"))
                ));
                assert_eq!(boundary.budget.snapshot().unwrap().used, charged);
            }
            gpu.device
                .poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(30)),
                })
                .unwrap();
            if fault != Fault::Unwind {
                assert_eq!(
                    boundary.budget.snapshot().unwrap().used,
                    charged,
                    "completion withheld after real native completion {fault:?}"
                );
                let done = boundary.held.lock().unwrap().take().unwrap();
                done();
            }
            assert_eq!(
                boundary.budget.snapshot().unwrap().used,
                0,
                "released {fault:?}"
            );
            drop(gpu.frame(hooks).unwrap().unwrap());
            assert_eq!(boundary.budget.snapshot().unwrap().used, 0);
        }
    }
}
