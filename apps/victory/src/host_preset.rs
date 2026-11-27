//! The host preset: what this PC is, looked at once before the title starts,
//! and what follows from it on a PC smaller than the one this fork is
//! measured on (64 GB of RAM, a 24 GB AMD card, 24 logical processors), where
//! none of it changes anything ([`decide`]):
//!
//! - The console profile when `--console` is not given (`auto`): Series S on
//!   a PC with 24 GiB of RAM or less or a card with 10 GiB of its own or
//!   less, Series X otherwise, and always with FSR 4K ([`choose_console`]).
//! - Defaults for switches that are there already, each set only where it is
//!   not set and said once with its reason ([`defaults`]). A value set by
//!   hand always stays.
//! - Warnings, which refuse nothing: a commit limit too low for a run on a PC
//!   with little RAM, and a render size whose tile lists a card other than
//!   AMD's runs cut ([`warnings`]).
//!
//! `VICTORY_HOST_PRESET=0`: none of it (Series X unless `--console` says
//! otherwise, no defaults, as before the preset); what the PC is and the
//! warnings are still said. Anything the look at the PC could not tell (no
//! adapter, no memory figures) leaves what depends on it as before.

use victory_hle::console::Console as Profile;
use victory_loader::resolution::{parse_output, parse_render_scale};

use crate::Console;

const GIB: u64 = 1 << 30;

/// A PC with little RAM: `GlobalMemoryStatusEx`'s `ullTotalPhys` of 24 GiB
/// or less (a 16 GB PC reports 15.6 to 15.8 GiB; a 32 GB one is not). A run
/// takes about 30 GB of private memory now (README), and Windows 3 to 4 GB
/// of the RAM.
const SMALL_RAM: u64 = 24 * GIB;

/// A card with little memory of its own, before a device exists:
/// `DXGI_ADAPTER_DESC1.DedicatedVideoMemory` of 10 GiB or less (8 and 10 GB
/// cards; a 12 GB one is not).
const SMALL_VRAM: u64 = 10 * GIB;

/// A processor with fewer logical processors than this
/// (`std::thread::available_parallelism`): 12 on a six-core Ryzen, 24 on the
/// i9-12900K this fork is measured on.
const SMALL_CPU: usize = 16;

/// AMD's PCI vendor id.
const AMD: u32 = 0x1002;

/// The commit a run wants beyond the RAM: its private memory is about 30 GB
/// (README), on top of what Windows and other programs hold.
const COMMIT_ABOVE_RAM: u64 = 24 * GIB;

/// The most groups an indirect dispatch runs on a card other than AMD's
/// (`victory-gpu` `indirect.rs`, `widest`: 65,535 is the most D3D12 names
/// for a dispatch). The title's tile lists are dispatches of one group per
/// tile of 8x4 pixels, so past 65,535 tiles (2,097,120 pixels) of a render
/// size the last ones can be cut: blocky patches, the lowest of the picture.
const MOST_GROUPS: u64 = 65_535;

/// What the PC is, as far as the preset goes by it; `None` where it could
/// not be told.
#[derive(Clone, Debug, Default)]
pub(crate) struct Probe {
    pub memory: Option<Memory>,
    /// The adapter the runtime makes its device on.
    pub adapter: Option<Adapter>,
    /// Logical processors (`std::thread::available_parallelism`; 0: not known).
    pub logical: usize,
    /// Physical cores (`GetLogicalProcessorInformationEx`).
    pub cores: Option<usize>,
    pub avx512: bool,
    /// Windows' major, minor and build numbers.
    pub windows: Option<[u32; 3]>,
}

/// `GlobalMemoryStatusEx`, in bytes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Memory {
    /// `ullTotalPhys`: the RAM Windows has.
    pub total: u64,
    /// `ullTotalPageFile`: the commit limit, the RAM and the paging file.
    pub commit_limit: u64,
    /// `ullAvailPageFile`: what of it is not committed.
    pub commit_free: u64,
}

/// `DXGI_ADAPTER_DESC1`.
#[derive(Clone, Debug)]
pub(crate) struct Adapter {
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    /// `DedicatedVideoMemory`, in bytes.
    pub dedicated: u64,
}

impl Probe {
    fn small_ram(&self) -> bool {
        self.memory.is_some_and(|m| m.total <= SMALL_RAM)
    }

    fn small_vram(&self) -> bool {
        self.adapter
            .as_ref()
            .is_some_and(|a| a.dedicated <= SMALL_VRAM)
    }

    fn small_cpu(&self) -> bool {
        self.logical != 0 && self.logical < SMALL_CPU
    }

    /// Known to be another vendor's card than AMD's (not known: no).
    fn not_amd(&self) -> bool {
        self.adapter.as_ref().is_some_and(|a| a.vendor != AMD)
    }
}

/// What the command line asks for.
pub(crate) struct Asked {
    /// `--console`; `Auto` when it is not given.
    pub console: Console,
    /// FSR 4K's input size, when `VICTORY_FSR_4K` asks for it.
    pub fsr_input: Option<[u32; 2]>,
}

/// What the preset makes of the PC.
#[derive(Debug)]
pub(crate) struct Decisions {
    pub console: Profile,
    /// Why that profile, for the console line (`auto` only).
    pub console_why: Option<String>,
    /// `VICTORY_HOST_PRESET=0`.
    pub off: bool,
    /// The switches to set: name, value, why.
    pub set: Vec<(&'static str, &'static str, &'static str)>,
    /// Switches the preset has a default for that are set already: name, the
    /// value they have, the preset's.
    pub kept: Vec<(&'static str, String, &'static str)>,
    pub warnings: Vec<String>,
}

/// The preset for `probe`, with the switches as `env` reads them (a name to
/// its value, `None` where it is not set). `VICTORY_HOST_PRESET=0` (on
/// without it): Series X where `--console` is not given and no defaults, as
/// before the preset, to tell whether something seen on a smaller PC comes
/// from one of them.
pub(crate) fn decide(
    probe: &Probe,
    asked: &Asked,
    env: &dyn Fn(&str) -> Option<String>,
) -> Decisions {
    let off = env("VICTORY_HOST_PRESET").is_some_and(|v| v.trim() == "0");
    let (console, console_why) =
        choose_console(asked.console, off, asked.fsr_input.is_some(), probe);
    let (mut set, mut kept) = (Vec::new(), Vec::new());
    if !off {
        for (name, value, why) in defaults(probe, console) {
            match env(name) {
                Some(theirs) => kept.push((name, theirs, value)),
                None => set.push((name, value, why)),
            }
        }
    }
    let with_set =
        |name: &str| env(name).or_else(|| set.iter().find(|s| s.0 == name).map(|s| s.1.to_owned()));
    let warnings = warnings(probe, console, asked.fsr_input, &with_set);
    Decisions {
        console,
        console_why,
        off,
        set,
        kept,
        warnings,
    }
}

/// The console profile, and why when the preset chose it.
///
/// Series S gives the title 7.6 GiB of console memory instead of 13.5
/// (`victory-gpu` `console.rs`) and makes it render for a 1080p output
/// (1280x720 before it scales up) instead of 4K (2560x1440): less of the PC's
/// memory and of the card's. FSR 4K needs the Series X 4K profile
/// (`fsr_4k.rs`).
fn choose_console(
    asked: Console,
    off: bool,
    fsr: bool,
    probe: &Probe,
) -> (Profile, Option<String>) {
    let why = match asked {
        Console::SeriesX => return (Profile::SeriesX, None),
        Console::SeriesS => return (Profile::SeriesS, None),
        Console::Auto if off => {
            return (
                Profile::SeriesX,
                Some("VICTORY_HOST_PRESET=0: as before".to_owned()),
            );
        }
        Console::Auto if fsr => {
            return (
                Profile::SeriesX,
                Some("FSR 4K is asked for, which needs the Series X 4K profile".to_owned()),
            );
        }
        Console::Auto => match (&probe.memory, &probe.adapter) {
            (Some(m), _) if probe.small_ram() => {
                format!("{} of RAM, 24 GiB or less", gib(m.total))
            }
            (_, Some(a)) if probe.small_vram() => {
                format!(
                    "a card with {} of its own, 10 GiB or less",
                    gib(a.dedicated)
                )
            }
            _ => return (Profile::SeriesX, Some("the default".to_owned())),
        },
    };
    (
        Profile::SeriesS,
        Some(format!(
            "{why}: the title gets 7.6 GiB of console memory instead of 13.5 and renders \
             for 1080p instead of 4K"
        )),
    )
}

/// The defaults that follow from the PC and the profile: name, value, why.
/// Each is a switch read where its doc comment says what it does.
fn defaults(probe: &Probe, console: Profile) -> Vec<(&'static str, &'static str, &'static str)> {
    let mut out = Vec::new();
    if probe.small_ram() {
        out.extend([
            (
                "VICTORY_RESIDENCY",
                "1",
                "the memory budgets of a smaller PC (victory-gpu residency.rs): texture \
                 prefetch and idle upload storage held back when free RAM or commit runs low",
            ),
            (
                "VICTORY_BURY_LAG",
                "64",
                "objects let go are dropped 64 submissions late, as on a larger PC: the budgets \
                 of a smaller PC would make it 16, and 64 was chosen after device losses in \
                 memory let go too early (victory-gpu host.rs)",
            ),
            (
                "VICTORY_WINDOW_KB",
                "1024",
                "copies of title memory outside buffers cut on a 1 MiB grid instead of 4 MiB: \
                 803 windows held 3.3 GB where what was asked for lay within 1.6 GB (run 60)",
            ),
            (
                "VICTORY_WINDOW_TAILS",
                "1",
                "a window asked for without a length begins at the ask, not at the 4 MiB line \
                 below it (2,052 of 3,236 MB of windows were asked for that way, run 65)",
            ),
        ]);
    }
    if probe.small_vram() {
        out.push((
            "VICTORY_LEVELS_ON_DEMAND",
            "1",
            "host copies of title textures take the card's memory level by level (2.2 to 3.1 GB \
             less on an NVIDIA card; only where the card has tiled resources tier 2, \
             victory-gpu tiles.rs)",
        ));
    }
    if probe.not_amd() {
        out.push((
            "VICTORY_BOOK",
            "0",
            "no book of who had what memory of the card: it decodes AMD's descriptors only, \
             says nothing on this card and costs memory (victory-gpu book.rs)",
        ));
        if console == Profile::SeriesS {
            out.push((
                "VICTORY_UPSCALER",
                "6",
                "the title's own upscaler, at the same 2/3 scale (1280x720 for 1080p): Series \
                 S's is the title's console FidelityFX, whose shaders have only been run on \
                 AMD's cards, not on 32-lane waves (victory-loader resolution.rs; 7 keeps \
                 FidelityFX)",
            ));
        }
    }
    out
}

/// What the title renders at before it scales up to its output, as far as
/// the switches tell (`victory-loader` `resolution.rs`): FSR 4K's input, or
/// the output (`VICTORY_OUTPUT`, else 3840x2160 on Series X and 1920x1080
/// on Series S) times its upscaler's scale: the title's own (kind 6, Series
/// X's) at `VICTORY_RENDER_SCALE` or 2/3, FidelityFX (kind 7, Series S's) at
/// `VICTORY_FFX_SCALE` or 2/3 on Series S, 1.0 on Series X.
fn render_size(
    console: Profile,
    fsr_input: Option<[u32; 2]>,
    env: &dyn Fn(&str) -> Option<String>,
) -> [u32; 2] {
    if let Some(size) = fsr_input {
        return size;
    }
    let series_s = console == Profile::SeriesS;
    let (w, h) = env("VICTORY_OUTPUT")
        .and_then(|v| parse_output(&v))
        .unwrap_or(if series_s { (1920, 1080) } else { (3840, 2160) });
    let kind = env("VICTORY_UPSCALER")
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|k| matches!(k, 6 | 7))
        .unwrap_or(if series_s { 7 } else { 6 });
    let scale = if kind == 6 {
        env("VICTORY_RENDER_SCALE")
            .and_then(|v| parse_render_scale(&v))
            .unwrap_or(2.0 / 3.0)
    } else {
        let stock = if series_s { 2.0 / 3.0 } else { 1.0 };
        env("VICTORY_FFX_SCALE")
            .and_then(|v| parse_render_scale(&v))
            .unwrap_or(stock)
    };
    [(w as f32 * scale) as u32, (h as f32 * scale) as u32]
}

/// The tiles of 8x4 pixels of a `[w, h]` picture.
fn tiles([w, h]: [u32; 2]) -> u64 {
    u64::from(w.div_ceil(8)) * u64::from(h.div_ceil(4))
}

/// The warnings, which refuse nothing: on a PC with little RAM a commit
/// limit under the RAM and [`COMMIT_ABOVE_RAM`], and on a card other than
/// AMD's a render size of more tiles than [`MOST_GROUPS`].
fn warnings(
    probe: &Probe,
    console: Profile,
    fsr_input: Option<[u32; 2]>,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    if probe.small_ram()
        && let Some(m) = probe.memory
        && m.commit_limit < m.total + COMMIT_ABOVE_RAM
    {
        out.push(format!(
            "the commit limit is {} ({} of RAM and about {} of paging file; {} not committed), and \
             a run takes about 30 GB of private memory: {} or more is wanted. Set a fixed paging \
             file of 32768 MB or more on an SSD (System > Advanced system settings > Performance \
             > Virtual memory), and close browsers and launchers before playing",
            gib(m.commit_limit),
            gib(m.total),
            gib(m.commit_limit.saturating_sub(m.total)),
            gib(m.commit_free),
            gib(m.total + COMMIT_ABOVE_RAM),
        ));
    }
    if probe.not_amd() {
        let size = render_size(console, fsr_input, env);
        let tiles = tiles(size);
        if tiles > MOST_GROUPS {
            let lower = if fsr_input.is_some() {
                "VICTORY_FSR_4K=performance (play.ps1 -Fsr4K performance: 1920x1080 in)"
            } else {
                "--console series-s (play.ps1 -Console series-s: 1280x720), or on Series X \
                 VICTORY_OUTPUT=2560x1440 (1706x960) or VICTORY_OUTPUT=1920x1080 (1280x720)"
            };
            out.push(format!(
                "the title renders at {}x{}, {tiles} tiles of 8x4 pixels, and on a card other than \
                 AMD's an indirect dispatch runs {MOST_GROUPS} groups at most: the title's tile \
                 lists (a group a tile) can be cut, the lowest part of the picture in blocky \
                 patches. Lower it with {lower}",
                size[0], size[1],
            ));
        }
    }
    out
}

fn gib(bytes: u64) -> String {
    format!("{:.1} GiB", bytes as f64 / GIB as f64)
}

/// The host line: everything the probe found.
fn describe(probe: &Probe) -> String {
    let memory = match probe.memory {
        Some(m) => format!(
            "{} of RAM, commit limit {} ({} not committed)",
            gib(m.total),
            gib(m.commit_limit),
            gib(m.commit_free)
        ),
        None => "RAM not known".to_owned(),
    };
    let adapter = match &probe.adapter {
        Some(a) => format!(
            "{} (vendor {:#06x}, device {:#06x}, {} of its own)",
            a.name,
            a.vendor,
            a.device,
            gib(a.dedicated)
        ),
        None => "no hardware adapter found".to_owned(),
    };
    let cores = match probe.cores {
        Some(cores) => format!(" on {cores} cores"),
        None => String::new(),
    };
    let avx512 = if probe.avx512 {
        "AVX-512"
    } else {
        "no AVX-512"
    };
    let windows = match probe.windows {
        Some([major, minor, build]) => format!("Windows {major}.{minor}.{build}"),
        None => "Windows version not known".to_owned(),
    };
    format!(
        "{memory}; {adapter}; {} logical processors{cores}, {avx512}; {windows}",
        probe.logical
    )
}

/// What of the shared definitions the PC is, for the preset's first line.
fn classes(probe: &Probe) -> String {
    let mut found = Vec::new();
    if probe.small_ram() {
        found.push("24 GiB of RAM or less");
    }
    if probe.small_vram() {
        found.push("a card with 10 GiB or less");
    }
    if probe.not_amd() {
        found.push("not an AMD card");
    }
    if probe.small_cpu() {
        found.push("fewer than 16 logical processors");
    }
    if found.is_empty() {
        "none of a smaller PC's defaults apply".to_owned()
    } else {
        found.join(", ")
    }
}

/// Looks at the PC, says what it is, sets the defaults that follow
/// ([`decide`]) and says the warnings; returns the console profile. Called
/// first thing by `run`, before anything reads the switches it sets.
#[cfg(windows)]
pub(crate) fn apply(console: Console, fsr_input: Option<[u32; 2]>) -> Profile {
    let probe = probe();
    let line = describe(&probe);
    tracing::info!(
        ram_mib = probe.memory.map(|m| m.total >> 20),
        commit_limit_mib = probe.memory.map(|m| m.commit_limit >> 20),
        commit_free_mib = probe.memory.map(|m| m.commit_free >> 20),
        adapter = probe.adapter.as_ref().map(|a| a.name.as_str()),
        vendor = probe.adapter.as_ref().map(|a| format!("{:#06x}", a.vendor)),
        device = probe.adapter.as_ref().map(|a| format!("{:#06x}", a.device)),
        vram_mib = probe.adapter.as_ref().map(|a| a.dedicated >> 20),
        logical = probe.logical,
        cores = probe.cores,
        avx512 = probe.avx512,
        windows = probe.windows.map(|[a, b, c]| format!("{a}.{b}.{c}")),
        "host: {line}"
    );
    println!("host           {line}");
    let env = |name: &str| std::env::var_os(name).map(|v| v.to_string_lossy().into_owned());
    let decided = decide(&probe, &Asked { console, fsr_input }, &env);
    if decided.off {
        println!("host preset    off (VICTORY_HOST_PRESET=0): no defaults for this PC");
    } else {
        println!(
            "host preset    {} (VICTORY_HOST_PRESET=0: none of what follows)",
            classes(&probe)
        );
    }
    for &(name, value, why) in &decided.set {
        // SAFETY: on Windows the environment may be set from any thread, and
        // nothing has read these switches yet: no title code has run, and
        // the runtime reads them when the title first needs them.
        unsafe { std::env::set_var(name, value) };
        tracing::info!(
            switch = name,
            value,
            why,
            "host preset: a default for this PC"
        );
        println!("host preset    {name}={value}: {why}");
    }
    for (name, theirs, ours) in &decided.kept {
        tracing::info!(
            switch = name,
            value = theirs,
            preset = ours,
            "host preset: set already, kept"
        );
        // (A title that relaunches itself hands its process the same ones.)
        if theirs == ours {
            println!("host preset    {name}={theirs}: set already");
        } else {
            println!("host preset    {name}={theirs} as set (the preset's default here: {ours})");
        }
    }
    if let Some(why) = &decided.console_why {
        tracing::info!(
            console = decided.console.name(),
            why,
            "console profile chosen (auto)"
        );
        println!(
            "console        auto: {}, {why} (--console series-x or series-s, play.ps1 -Console, \
             chooses)",
            decided.console.name()
        );
    }
    for warning in &decided.warnings {
        tracing::warn!("host preset: {warning}");
        println!("warning        {warning}");
    }
    decided.console
}

/// [`Probe`] of this PC.
#[cfg(windows)]
fn probe() -> Probe {
    #[cfg(target_arch = "x86_64")]
    let avx512 = std::arch::is_x86_feature_detected!("avx512f");
    #[cfg(not(target_arch = "x86_64"))]
    let avx512 = false;
    Probe {
        memory: memory(),
        adapter: adapter(),
        logical: std::thread::available_parallelism().map_or(0, |n| n.get()),
        cores: cores(),
        avx512,
        windows: windows_version(),
    }
}

#[cfg(windows)]
fn memory() -> Option<Memory> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    // SAFETY: a structure of the size it says, which the call fills.
    unsafe { GlobalMemoryStatusEx(&mut status) }.ok()?;
    Some(Memory {
        total: status.ullTotalPhys,
        commit_limit: status.ullTotalPageFile,
        commit_free: status.ullAvailPageFile,
    })
}

/// The adapter the runtime makes its device on, as `victory-shader`'s
/// `create_preferred_device` picks it: with `VICTORY_D3D12_ADAPTER`, the
/// first hardware adapter of that index or name; else the first hardware
/// adapter in DXGI's high-performance order, else DXGI's first. No device is
/// made here: the runtime sets the debug layer and crash tracing up before it
/// makes its own (`VICTORY_D3D12_DEBUG`, `VICTORY_D3D12_DRED`). So where the
/// first hardware adapter in that order cannot make a shader model 6.6
/// device, the runtime goes on to the next one and this still names the
/// first.
#[cfg(windows)]
fn adapter() -> Option<Adapter> {
    use victory_shader::emit::host::{adapter_matches, software};
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory2, DXGI_ADAPTER_DESC1, DXGI_CREATE_FACTORY_FLAGS,
        DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE, IDXGIAdapter1, IDXGIFactory4, IDXGIFactory6,
    };
    use windows::core::Interface;
    let name = |desc: &DXGI_ADAPTER_DESC1| {
        String::from_utf16_lossy(&desc.Description)
            .trim_end_matches('\0')
            .to_owned()
    };
    let described = |desc: &DXGI_ADAPTER_DESC1| Adapter {
        name: name(desc),
        vendor: desc.VendorId,
        device: desc.DeviceId,
        dedicated: desc.DedicatedVideoMemory as u64,
    };
    let requested = std::env::var("VICTORY_D3D12_ADAPTER").unwrap_or_default();
    let requested = requested.trim();
    // SAFETY: plain adapter queries.
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
        if !requested.is_empty() {
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapters1(index) {
                let desc = adapter.GetDesc1().ok()?;
                if !software(&desc) && adapter_matches(requested, index, &name(&desc)) {
                    return Some(described(&desc));
                }
                index += 1;
            }
            return None;
        }
        if let Ok(factory) = factory.cast::<IDXGIFactory6>() {
            let mut index = 0;
            while let Ok(adapter) = factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(
                index,
                DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
            ) {
                if let Ok(desc) = adapter.GetDesc1()
                    && !software(&desc)
                {
                    return Some(described(&desc));
                }
                index += 1;
            }
        }
        // DXGI's default adapter, which the runtime's last try takes.
        let desc = factory.EnumAdapters1(0).ok()?.GetDesc1().ok()?;
        (!software(&desc)).then(|| described(&desc))
    }
}

/// Physical cores: the processor-core records of
/// `GetLogicalProcessorInformationEx`.
#[cfg(windows)]
fn cores() -> Option<usize> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
    };
    let mut bytes = 0u32;
    // SAFETY: a size query (it fails, saying the size).
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut bytes) };
    if bytes == 0 {
        return None;
    }
    let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
    // SAFETY: the buffer holds `bytes` bytes, aligned for the records.
    unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            Some(buffer.as_mut_ptr().cast()),
            &mut bytes,
        )
    }
    .ok()?;
    let (mut found, mut at) = (0, 0usize);
    // Each record starts with its relationship and its size, two u32s.
    while at + 8 <= bytes as usize {
        // SAFETY: eight bytes inside the buffer.
        let [_, size] = unsafe {
            buffer
                .as_ptr()
                .cast::<u8>()
                .add(at)
                .cast::<[u32; 2]>()
                .read_unaligned()
        };
        if size == 0 {
            break;
        }
        found += 1;
        at += size as usize;
    }
    Some(found)
}

#[cfg(windows)]
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    /// The system's version as it is (`GetVersionExW` tells a program
    /// without a manifest 6.2, build 9200): an NTSTATUS, 0 when filled.
    fn RtlGetVersion(info: *mut windows::Win32::System::SystemInformation::OSVERSIONINFOW) -> i32;
}

#[cfg(windows)]
fn windows_version() -> Option<[u32; 3]> {
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
        ..Default::default()
    };
    // SAFETY: a structure of the size it says, which the call fills.
    let status = unsafe { RtlGetVersion(&mut info) };
    (status == 0).then_some([info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber])
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    /// The PC this fork is measured on: i9-12900K, 64 GB, RX 7900 XTX.
    fn dev_box() -> Probe {
        Probe {
            memory: Some(Memory {
                total: 63_850 * MIB,
                commit_limit: 73_000 * MIB,
                commit_free: 50_000 * MIB,
            }),
            adapter: Some(Adapter {
                name: "AMD Radeon RX 7900 XTX".to_owned(),
                vendor: AMD,
                device: 0x744c,
                dedicated: 24_560 * MIB,
            }),
            logical: 24,
            cores: Some(16),
            avx512: false,
            windows: Some([10, 0, 26100]),
        }
    }

    /// Ryzen 5 7600X, 16 GB, RTX 5060 Ti 8 GB, a system-managed paging file.
    fn small_pc() -> Probe {
        Probe {
            memory: Some(Memory {
                total: 16_100 * MIB,
                commit_limit: 18_600 * MIB,
                commit_free: 9_000 * MIB,
            }),
            adapter: Some(Adapter {
                name: "NVIDIA GeForce RTX 5060 Ti".to_owned(),
                vendor: 0x10de,
                device: 0x2d04,
                dedicated: 8_000 * MIB,
            }),
            logical: 12,
            cores: Some(6),
            avx512: true,
            windows: Some([10, 0, 26100]),
        }
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn auto() -> Asked {
        Asked {
            console: Console::Auto,
            fsr_input: None,
        }
    }

    fn names(d: &Decisions) -> Vec<&'static str> {
        d.set.iter().map(|s| s.0).collect()
    }

    #[test]
    fn the_dev_box_is_left_as_it_was() {
        let d = decide(&dev_box(), &auto(), &none);
        assert_eq!(d.console, Profile::SeriesX);
        assert!(
            d.set.is_empty() && d.kept.is_empty() && d.warnings.is_empty(),
            "{d:?}"
        );
        assert_eq!(classes(&dev_box()), "none of a smaller PC's defaults apply");
        // Asked for by name, as before.
        let s = Asked {
            console: Console::SeriesS,
            fsr_input: None,
        };
        let d = decide(&dev_box(), &s, &none);
        assert_eq!(d.console, Profile::SeriesS);
        assert!(d.console_why.is_none() && d.set.is_empty() && d.warnings.is_empty());
    }

    #[test]
    fn a_small_pc_gets_series_s_and_the_defaults() {
        let d = decide(&small_pc(), &auto(), &none);
        assert_eq!(d.console, Profile::SeriesS);
        assert!(
            d.console_why
                .as_deref()
                .is_some_and(|w| w.contains("of RAM"))
        );
        assert_eq!(
            names(&d),
            [
                "VICTORY_RESIDENCY",
                "VICTORY_BURY_LAG",
                "VICTORY_WINDOW_KB",
                "VICTORY_WINDOW_TAILS",
                "VICTORY_LEVELS_ON_DEMAND",
                "VICTORY_BOOK",
                "VICTORY_UPSCALER",
            ]
        );
        let value = |name| d.set.iter().find(|s| s.0 == name).map(|s| s.1);
        assert_eq!(value("VICTORY_BURY_LAG"), Some("64"));
        assert_eq!(value("VICTORY_WINDOW_KB"), Some("1024"));
        assert_eq!(value("VICTORY_UPSCALER"), Some("6"));
        assert_eq!(value("VICTORY_BOOK"), Some("0"));
        // 18.6 GiB of commit is short of 15.7 + 24: said, at 1280x720 nothing else.
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert!(d.warnings[0].contains("32768 MB"));
    }

    #[test]
    fn what_is_set_by_hand_stays() {
        let env = |name: &str| match name {
            "VICTORY_RESIDENCY" => Some("0".to_owned()),
            "VICTORY_UPSCALER" => Some("7".to_owned()),
            _ => None,
        };
        let d = decide(&small_pc(), &auto(), &env);
        assert!(!names(&d).contains(&"VICTORY_RESIDENCY"));
        assert!(!names(&d).contains(&"VICTORY_UPSCALER"));
        assert_eq!(
            d.kept,
            [
                ("VICTORY_RESIDENCY", "0".to_owned(), "1"),
                ("VICTORY_UPSCALER", "7".to_owned(), "6")
            ]
        );
        // An explicit profile wins as well.
        let x = Asked {
            console: Console::SeriesX,
            fsr_input: None,
        };
        let d = decide(&small_pc(), &x, &none);
        assert_eq!(d.console, Profile::SeriesX);
        assert!(!names(&d).contains(&"VICTORY_UPSCALER"));
    }

    #[test]
    fn the_preset_off_is_as_before() {
        let env = |name: &str| (name == "VICTORY_HOST_PRESET").then(|| "0".to_owned());
        let d = decide(&small_pc(), &auto(), &env);
        assert!(d.off);
        assert_eq!(d.console, Profile::SeriesX);
        assert!(d.set.is_empty() && d.kept.is_empty());
        // The warnings are still said: Series X renders 2560x1440 on an NVIDIA card.
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
        // Any other value leaves it on.
        let env = |name: &str| (name == "VICTORY_HOST_PRESET").then(|| "1".to_owned());
        assert!(!decide(&small_pc(), &auto(), &env).off);
    }

    #[test]
    fn nothing_known_is_as_before() {
        let d = decide(&Probe::default(), &auto(), &none);
        assert_eq!(d.console, Profile::SeriesX);
        assert!(d.set.is_empty() && d.warnings.is_empty());
        assert_eq!(
            classes(&Probe::default()),
            "none of a smaller PC's defaults apply"
        );
        let line = describe(&Probe::default());
        assert!(
            line.contains("RAM not known") && line.contains("no hardware adapter"),
            "{line}"
        );
    }

    #[test]
    fn fsr_4k_keeps_series_x() {
        let quality = Asked {
            console: Console::Auto,
            fsr_input: Some([2560, 1440]),
        };
        let d = decide(&small_pc(), &quality, &none);
        assert_eq!(d.console, Profile::SeriesX);
        assert!(!names(&d).contains(&"VICTORY_UPSCALER"));
        assert!(
            d.warnings
                .iter()
                .any(|w| w.contains("2560x1440") && w.contains("performance"))
        );
        let performance = Asked {
            console: Console::Auto,
            fsr_input: Some([1920, 1080]),
        };
        let d = decide(&small_pc(), &performance, &none);
        assert!(
            !d.warnings.iter().any(|w| w.contains("tile")),
            "{:?}",
            d.warnings
        );
    }

    #[test]
    fn the_thresholds() {
        let at = |total: u64, dedicated: u64, vendor: u32, logical: usize| Probe {
            memory: Some(Memory {
                total,
                commit_limit: total + 32 * GIB,
                commit_free: 16 * GIB,
            }),
            adapter: Some(Adapter {
                name: String::new(),
                vendor,
                device: 0,
                dedicated,
            }),
            logical,
            ..Probe::default()
        };
        let console = |p: &Probe| decide(p, &auto(), &none).console;
        // 24 GiB is small, a byte more is not; the same for 10 GiB of card.
        assert_eq!(console(&at(24 * GIB, 16 * GIB, AMD, 24)), Profile::SeriesS);
        assert_eq!(
            console(&at(24 * GIB + 1, 16 * GIB, AMD, 24)),
            Profile::SeriesX
        );
        assert_eq!(console(&at(32 * GIB, 10 * GIB, AMD, 24)), Profile::SeriesS);
        assert_eq!(
            console(&at(32 * GIB, 10 * GIB + 1, AMD, 24)),
            Profile::SeriesX
        );
        // A large AMD PC with a small card: only the card's default.
        let d = decide(&at(64 * GIB, 8 * GIB, AMD, 24), &auto(), &none);
        assert_eq!(names(&d), ["VICTORY_LEVELS_ON_DEMAND"]);
        // A large NVIDIA PC: no Series S, the book off, the 4K warning.
        let d = decide(&at(64 * GIB, 24 * GIB, 0x10de, 32), &auto(), &none);
        assert_eq!(d.console, Profile::SeriesX);
        assert_eq!(names(&d), ["VICTORY_BOOK"]);
        assert_eq!(d.warnings.len(), 1);
        // The commit limit is only looked at with little RAM, then against RAM + 24 GiB.
        let mut p = at(16 * GIB, 8 * GIB, AMD, 12);
        p.memory = Some(Memory {
            total: 16 * GIB,
            commit_limit: 40 * GIB,
            commit_free: GIB,
        });
        assert!(decide(&p, &auto(), &none).warnings.is_empty());
        p.memory = Some(Memory {
            total: 16 * GIB,
            commit_limit: 40 * GIB - 1,
            commit_free: GIB,
        });
        assert_eq!(decide(&p, &auto(), &none).warnings.len(), 1);
        assert!(at(0, 0, AMD, 15).small_cpu() && !at(0, 0, AMD, 16).small_cpu());
        assert!(!Probe::default().small_cpu());
    }

    #[test]
    fn render_sizes_as_the_switches_make_them() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|p| p.0 == name).map(|p| p.1.to_owned())
        };
        let size = |console, fsr, pairs| render_size(console, fsr, &env(pairs));
        assert_eq!(size(Profile::SeriesX, None, &[]), [2560, 1440]);
        assert_eq!(size(Profile::SeriesS, None, &[]), [1280, 720]);
        assert_eq!(
            size(Profile::SeriesS, None, &[("VICTORY_UPSCALER", "6")]),
            [1280, 720]
        );
        assert_eq!(
            size(
                Profile::SeriesX,
                Some([2258, 1270]),
                &[("VICTORY_OUTPUT", "1920x1080")]
            ),
            [2258, 1270]
        );
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_OUTPUT", "1920x1080")]),
            [1280, 720]
        );
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_OUTPUT", "2560x1440")]),
            [1706, 960]
        );
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_RENDER_SCALE", "1.0")]),
            [3840, 2160]
        );
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_UPSCALER", "7")]),
            [3840, 2160]
        );
        assert_eq!(
            size(
                Profile::SeriesS,
                None,
                &[("VICTORY_UPSCALER", "7"), ("VICTORY_FFX_SCALE", "1.0")]
            ),
            [1920, 1080]
        );
        // Not one of the title's modes, out of range: as unset.
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_OUTPUT", "1000x1000")]),
            [2560, 1440]
        );
        assert_eq!(
            size(Profile::SeriesX, None, &[("VICTORY_UPSCALER", "5")]),
            [2560, 1440]
        );
        // 1920x1080 is 64,800 tiles, 2048x1024 one more than the most.
        assert_eq!(tiles([1920, 1080]), 64_800);
        assert_eq!(tiles([2560, 1440]), 115_200);
        assert_eq!(tiles([2048, 1024]), MOST_GROUPS + 1);
        assert_eq!(tiles([2047, 1023]), MOST_GROUPS + 1);
    }

    #[test]
    fn series_x_on_another_card_is_warned_of_until_its_size_is_lowered() {
        let x = Asked {
            console: Console::SeriesX,
            fsr_input: None,
        };
        let warned = |output: Option<&'static str>| {
            let env = move |name: &str| {
                (name == "VICTORY_OUTPUT")
                    .then_some(output)
                    .flatten()
                    .map(str::to_owned)
            };
            decide(&small_pc(), &x, &env)
                .warnings
                .iter()
                .any(|w| w.contains("tile"))
        };
        assert!(warned(None));
        assert!(!warned(Some("2560x1440")));
        assert!(!warned(Some("1920x1080")));
        // On the dev box's AMD card none.
        assert!(decide(&dev_box(), &x, &none).warnings.is_empty());
    }

    #[test]
    fn the_host_line_has_it_all() {
        let line = describe(&small_pc());
        for part in [
            "15.7 GiB of RAM",
            "NVIDIA GeForce RTX 5060 Ti",
            "0x10de",
            "7.8 GiB of its own",
            "12 logical processors on 6 cores",
            "AVX-512",
            "Windows 10.0.26100",
        ] {
            assert!(line.contains(part), "{part} not in {line}");
        }
        assert_eq!(
            classes(&small_pc()),
            "24 GiB of RAM or less, a card with 10 GiB or less, not an AMD card, fewer than 16 logical processors"
        );
    }
}
