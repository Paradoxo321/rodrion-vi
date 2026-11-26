//! Keeps the GPU clocks awake between CPU-bound submissions (`VICTORY_CARD_AWAKE=1`;
//! off by default). Ported from codec-d/cool_fox (Willy).
//!
//! When CPU prepare takes 100+ ms between short 15 ms GPU bursts, AMD/NVIDIA drivers
//! downclock the GPU and idle the command processor. A tiny 64-thread compute pulse
//! on an async compute queue keeps clocks boosted and auto-pauses after 250 ms idle.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
use windows::core::Interface;

/// ~2 ms pulse on a single 64-thread compute group.
const COUNT: &str = r#"
#define RS "UAV(u0)"
RWByteAddressBuffer sum : register(u0);
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint x = id.x;
    [loop] for (uint i = 0; i < 400000u; i++) x = x * 1664525u + 1013904223u;
    sum.Store(id.x * 4u, x);
}
"#;

/// Auto-sleep after this many ms with no title submission.
const QUIET_MS: u64 = 250;

fn now_ms() -> u64 {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

static LAST: AtomicU64 = AtomicU64::new(0);

fn active_at(last: u64, now: u64) -> bool {
    last != 0 && now + 1 < last + QUIET_MS
}

fn wanted() -> bool {
    active_at(LAST.load(Ordering::Relaxed), now_ms())
}

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CARD_AWAKE").is_some_and(|v| v == "1"))
}

/// Called whenever a submission goes to the drawing queue.
pub(super) fn sent(device: &ID3D12Device) {
    if !on() {
        return;
    }
    LAST.store(now_ms() + 1, Ordering::Relaxed);
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        tracing::info!("keeping GPU clocks awake between submissions (VICTORY_CARD_AWAKE=1)");
        std::mem::forget(start(device, wanted));
    });
}

pub(crate) struct Awake {
    stop: std::sync::Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn start(device: &ID3D12Device, wanted: impl Fn() -> bool + Send + 'static) -> Option<Awake> {
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let (device, ended) = (device.clone(), stop.clone());
    let thread = std::thread::Builder::new()
        .name("victory-card-awake".into())
        .spawn(move || {
            if let Err(e) = run(&device, &wanted, &ended) {
                tracing::warn!(error = %e, "the card is not kept awake");
            }
        })
        .ok()?;
    Some(Awake {
        stop,
        thread: Some(thread),
    })
}

fn run(device: &ID3D12Device, wanted: &dyn Fn() -> bool, stop: &AtomicBool) -> windows::core::Result<()> {
    let code = super::compile(COUNT, windows::core::s!("main"), windows::core::s!("cs_5_1"))?;
    // SAFETY: plain D3D12 object creation on live device; objects outlive all queued work.
    unsafe {
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &code)?;
        let pso: ID3D12PipelineState = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: code.as_ptr().cast(),
                BytecodeLength: code.len(),
            },
            ..Default::default()
        })?;
        let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_COMPUTE,
            Priority: D3D12_COMMAND_QUEUE_PRIORITY_NORMAL.0,
            ..Default::default()
        })?;
        let mut sum: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: 256,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                ..Default::default()
            },
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut sum,
        )?;
        let sum = sum.ok_or_else(windows::core::Error::empty)?;
        let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COMPUTE)?;
        let mut lists = Vec::new();
        for _ in 0..2 {
            let list: ID3D12GraphicsCommandList =
                device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_COMPUTE, &allocator, &pso)?;
            list.SetComputeRootSignature(&root);
            list.SetComputeRootUnorderedAccessView(0, sum.GetGPUVirtualAddress());
            list.Dispatch(1, 1, 1);
            list.Close()?;
            lists.push(Some(list.cast::<ID3D12CommandList>()?));
        }
        let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
        let event = CreateEventW(None, false, false, None)?;
        let mut value = 0u64;
        let mut result = Ok(());
        while !stop.load(Ordering::Relaxed) && result.is_ok() {
            if !wanted() {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            if value >= 2 && fence.GetCompletedValue() + 2 <= value {
                result = fence.SetEventOnCompletion(value - 1, event);
                if result.is_ok() {
                    WaitForSingleObject(event, 1000);
                    result = device.GetDeviceRemovedReason();
                }
                continue;
            }
            queue.ExecuteCommandLists(&lists[(value % 2) as usize..][..1]);
            value += 1;
            result = queue.Signal(&fence, value);
        }
        if result.is_ok() && fence.GetCompletedValue() < value && fence.SetEventOnCompletion(value, event).is_ok() {
            WaitForSingleObject(event, INFINITE);
        }
        let _ = CloseHandle(event);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awake_auto_sleeps_after_quiet_window() {
        assert!(!active_at(0, 100));
        assert!(active_at(100, 100));
        assert!(active_at(100, 348));
        assert!(!active_at(100, 349));
    }
}
