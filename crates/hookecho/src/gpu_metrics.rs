//! Optional map draw timestamps. Readback is asynchronous; unsupported devices keep rendering.
use std::collections::{HashMap, VecDeque};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub const FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES)
    .union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);
static MEMORY: Mutex<(u64, u64)> = Mutex::new((0, 0));
pub fn set_memory(gpu: u64, shadows: u64) {
    *MEMORY.lock().unwrap() = (gpu, shadows);
}
pub fn memory() -> (u64, u64) {
    *MEMORY.lock().unwrap()
}
static UPLOADS: Mutex<[u64; 4]> = Mutex::new([0; 4]);
/// Tokens identify radar data actually prepared, not merely queued by the UI.
pub fn set_uploads(tokens: [u64; 4]) { *UPLOADS.lock().unwrap() = tokens; }
pub fn uploads() -> [u64; 4] { *UPLOADS.lock().unwrap() }
pub fn upload_acknowledged(pane: usize, token: u64) -> bool {
    token != 0 && uploads().get(pane) == Some(&token)
}

static PAINTED: Mutex<[u64; 4]> = Mutex::new([0; 4]);
/// A map callback recorded this sweep into a render pass (not merely uploaded it).
pub fn mark_painted(pane: usize, token: u64) {
    if let Some(slot) = PAINTED.lock().unwrap().get_mut(pane) { *slot = token; }
}
pub fn painted_tokens() -> [u64; 4] { *PAINTED.lock().unwrap() }
pub fn next_upload_token() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

static SAMPLES: Mutex<Option<HashMap<u32, VecDeque<f32>>>> = Mutex::new(None);
static LOST: AtomicBool = AtomicBool::new(false);
pub fn report_device_loss() -> bool {
    !LOST.swap(true, Ordering::Relaxed)
}
pub fn device_lost() -> bool {
    LOST.load(Ordering::Relaxed)
}
static SUPPORTED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub fn reset_samples() {
    *SAMPLES.lock().unwrap() = None;
}
pub fn supported() -> bool {
    SUPPORTED.load(Ordering::Relaxed)
}
pub fn percentiles(pane: u32) -> Option<(f32, f32)> {
    let samples = SAMPLES.lock().ok()?;
    let mut values: Vec<_> = samples.as_ref()?.get(&pane)?.iter().copied().collect();
    values.sort_by(f32::total_cmp);
    Some((
        values[(values.len() - 1) / 2],
        values[((values.len() - 1) * 95).div_ceil(100)],
    ))
}
fn observe(pane: u32, ms: f32) {
    if !ms.is_finite() || ms < 0.0 {
        return;
    }
    let mut samples = SAMPLES.lock().unwrap();
    let values = samples
        .get_or_insert_with(HashMap::new)
        .entry(pane)
        .or_default();
    if values.len() == 120 {
        values.pop_front();
    }
    values.push_back(ms);
}

pub struct Timer {
    query: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    readback: wgpu::Buffer,
    written: AtomicBool,
    pending: Arc<AtomicBool>,
}
impl Timer {
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        let supported = device.features().contains(FEATURES);
        SUPPORTED.store(supported, Ordering::Relaxed);
        if !supported {
            return None;
        }
        Some(Self {
            query: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("map draw timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map timestamp resolve"),
                size: 256,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("map timestamp readback"),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
            written: AtomicBool::new(false),
            pending: Arc::new(AtomicBool::new(false)),
        })
    }
    pub fn begin_encoder(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.query, 0);
    }
    pub fn end_encoder(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.write_timestamp(&self.query, 1);
        self.written.store(true, Ordering::Relaxed);
    }
    pub fn begin(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.write_timestamp(&self.query, 0);
    }
    pub fn end(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.write_timestamp(&self.query, 1);
        self.written.store(true, Ordering::Relaxed);
    }
    /// Resolve the preceding draw before this frame overwrites the query slots.
    pub fn collect_into(&self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue, pane: u32) {
        if !self.written.load(Ordering::Relaxed) || self.pending.swap(true, Ordering::Relaxed) {
            return;
        }
        self.written.store(false, Ordering::Relaxed);
        encoder.resolve_query_set(&self.query, 0..2, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.readback, 0, 16);
        let buffer = self.readback.clone();
        let pending = self.pending.clone();
        let period = queue.get_timestamp_period();
        encoder.map_buffer_on_submit(&self.readback, wgpu::MapMode::Read, .., move |result| {
            if result.is_ok() {
                let bytes = buffer.get_mapped_range(..);
                let begin = u64::from_le_bytes(bytes[..8].try_into().unwrap());
                let end = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
                drop(bytes);
                buffer.unmap();
                if end >= begin {
                    observe(pane, (end - begin) as f32 * period / 1_000_000.0);
                }
            }
            pending.store(false, Ordering::Relaxed);
        });
    }
}
