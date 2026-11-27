//! Host-memory admission for speculative texture work and reusable upload storage.
//!
//! From codec-d/cool_fox (641835a4, fabb872a, c64fc9b4, 08e4574d, e94d277a; the
//! values of the last). Only with `VICTORY_RESIDENCY=1` ([`on`]): the memory
//! of the system is then asked once a second; below 3 GiB of free RAM or 4 GiB
//! of commit (out again above 5 / 6) texture prefetch is limited to 64 MiB a
//! frame and idle scratch and staging blocks are let go; below 1.5 GiB of
//! commit to 16 MiB; the texture pass stops below 512 MiB of commit, new
//! staging blocks for textures picked by index below 1 GiB. With 32 GiB of RAM
//! or less (or `VICTORY_RESIDENCY_LIMITED=1`) the budgets of a smaller PC.
//! Off, nothing here changes anything, except that for 500 ms after the card
//! refused memory (E_OUTOFMEMORY) the texture pass stops ([`upload_refused`]).
//!
//! Whatever `VICTORY_RESIDENCY` says: whether this is a PC of 24 GiB of
//! memory or less ([`small_ram_host`]), and how full the PC's memory the
//! card shares is ([`shared_band`], followed on such a PC or with a small
//! card only), by which less is kept spare for reuse ([`short_of_ram`]).
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

const MIB: u64 = 1 << 20;

/// `VICTORY_RESIDENCY=1`: the memory budgets of this module (off by default).
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_RESIDENCY").is_some_and(|v| v == "1"))
}
const GIB: u64 = 1 << 30;
static PRESSURED: AtomicBool = AtomicBool::new(false);
static HEADROOM: AtomicU64 = AtomicU64::new(u64::MAX);

fn memory_status() -> Option<MEMORYSTATUSEX> {
    // SAFETY: the API initializes the fields of a correctly sized structure.
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status)
}

// The texture sweep may run for seconds inside a single submitted frame.
static NEXT_COMMIT_SAMPLE: AtomicU64 = AtomicU64::new(0);
static AVAILABLE_COMMIT: AtomicU64 = AtomicU64::new(u64::MAX);
static RETRY_UPLOAD_AT: AtomicU64 = AtomicU64::new(0);

pub(crate) fn allocation_error(error: &windows::core::Error) {
    refused(error);
    if error.code() == windows::Win32::Foundation::E_OUTOFMEMORY {
        RETRY_UPLOAD_AT.fetch_max(crate::host::coarse_ms().saturating_add(500), Ordering::Relaxed);
    }
}

thread_local! {
    /// Whether this thread's last refused allocation was refused for want
    /// of memory ([`refused`]).
    static OUT_OF_MEMORY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Notes on this thread whether an allocation was refused for want of
/// memory (E_OUTOFMEMORY): `crate::host::or_after_release` then lets go of
/// what is kept spare and asks once more, where the run would end.
pub(crate) fn refused(error: &windows::core::Error) {
    OUT_OF_MEMORY.set(error.code() == windows::Win32::Foundation::E_OUTOFMEMORY);
}

/// Whether [`refused`] noted a want of memory since the last call; forgets it.
pub(crate) fn take_out_of_memory() -> bool {
    OUT_OF_MEMORY.replace(false)
}

thread_local! {
    static STAGING_DEFERRED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn note_staging_deferred() {
    STAGING_DEFERRED.set(true);
}

pub(crate) fn take_staging_deferred() -> bool {
    STAGING_DEFERRED.replace(false)
}

pub(crate) fn upload_refused() -> bool {
    crate::host::coarse_ms() < RETRY_UPLOAD_AT.load(Ordering::Relaxed)
}

fn sample_commit(now: u64) -> u64 {
    let next = NEXT_COMMIT_SAMPLE.load(Ordering::Relaxed);
    if now >= next
        && NEXT_COMMIT_SAMPLE
            .compare_exchange(next, now.saturating_add(32), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        && let Some(status) = memory_status()
    {
        AVAILABLE_COMMIT.store(status.ullAvailPageFile, Ordering::Relaxed);
    }
    AVAILABLE_COMMIT.load(Ordering::Relaxed)
}

fn has_upload_reserve(commit: u64, bake: bool) -> bool {
    // Keep a protective floor against Rust/game OOM aborts without starving
    // bindless world textures or one-shot ped decoration bakes when steady-state
    // commit headroom sits in the 1-4 GiB range on 32 GiB hosts.
    commit >= if bake { 256 * MIB } else { 512 * MIB }
}

fn has_growth_reserve(commit: u64, bake: bool) -> bool {
    // Growing a fresh 64 MiB upload block for speculative bindless prefetch
    // requires more headroom than reusing slices already committed inside the
    // staging pool.
    commit >= if bake { 256 * MIB } else { 1024 * MIB }
}

/// Leave commit for game allocations and submissions already in flight while
/// still letting bindless textures and one-shot material bakes upload.
pub(crate) fn admit_texture_pass(bake: bool) -> bool {
    let now = crate::host::coarse_ms();
    if now < RETRY_UPLOAD_AT.load(Ordering::Relaxed) {
        return false;
    }
    if !on() {
        return true;
    }
    has_upload_reserve(sample_commit(now), bake)
}

/// Whether the staging allocator may commit a brand-new OS upload buffer
/// (reusing existing free ranges in already-committed pool blocks stays open
/// whenever `admit_texture_pass` is true).
pub(crate) fn allow_staging_growth(bake: bool) -> bool {
    if !on() {
        return true;
    }
    let now = crate::host::coarse_ms();
    if now < RETRY_UPLOAD_AT.load(Ordering::Relaxed) {
        return false;
    }
    has_growth_reserve(sample_commit(now), bake)
}

pub(crate) fn limited_host_ram() -> bool {
    static LIMITED: OnceLock<bool> = OnceLock::new();
    *LIMITED.get_or_init(|| {
        on() && (std::env::var_os("VICTORY_RESIDENCY_LIMITED").is_some_and(|v| v == "1")
            || memory_status().is_some_and(|s| s.ullTotalPhys <= 32 * GIB))
    })
}

/// The PC's memory, in bytes (`GlobalMemoryStatusEx`'s total physical
/// memory; a PC of 16 GB says some 15.7 GiB); `None` if the system does not say.
pub(crate) fn total_ram() -> Option<u64> {
    static TOTAL: OnceLock<Option<u64>> = OnceLock::new();
    *TOTAL.get_or_init(|| memory_status().map(|s| s.ullTotalPhys))
}

/// Whether this is a PC of 24 GiB of memory or less: then less is kept
/// spare for reuse ([`short_of_ram`]), whether `VICTORY_RESIDENCY` is on or
/// not. The layer holds some 30 GB of private memory on a PC of 64 GB, and
/// what it keeps spare is sized for that one. `VICTORY_SMALL_RAM=1` takes
/// any PC for one, `=0` none, to measure what it changes on either kind.
/// (With a small card the memory the card shares is followed all the same,
/// [`shared_followed`], and from its Watch band on [`short_of_ram`] holds
/// again: a run with none of it is `VICTORY_SMALL_RAM=0` with
/// `VICTORY_SHARED_BAND=0`.)
pub(crate) fn small_ram_host() -> bool {
    static SMALL: OnceLock<bool> = OnceLock::new();
    *SMALL.get_or_init(|| match std::env::var("VICTORY_SMALL_RAM").as_deref().map(str::trim) {
        Ok("1") => true,
        Ok("0") => false,
        _ => total_ram().is_some_and(small_ram),
    })
}

/// [`small_ram_host`] for `total` bytes of memory.
fn small_ram(total: u64) -> bool {
    total <= 24 * GIB
}

/// How full the PC's memory the card shares is, by the system's account
/// for this process (DXGI's non-local group: upload and read-back memory,
/// host copies of title memory that shaders read, and what the card's own
/// memory does not hold): its use against its budget ([`refresh_shared`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub(crate) enum Shared {
    Calm,
    Watch,
    High,
}

/// The band for `used` of `budget` bytes after `was`: Watch from 70 % of
/// the budget, High from 85 %, each left only 5 points lower (High below
/// 80 %, Watch below 65 %), so that a use near a line does not change the
/// policy every second. No budget: as it was.
pub(crate) fn next_shared(was: Shared, used: u64, budget: u64) -> Shared {
    if budget == 0 {
        return was;
    }
    let permille = u128::from(used) * 1000 / u128::from(budget);
    let band = |watch: u128, high: u128| {
        if permille >= high {
            Shared::High
        } else if permille >= watch {
            Shared::Watch
        } else {
            Shared::Calm
        }
    };
    match was {
        Shared::Calm => band(700, 850),
        Shared::Watch => band(650, 850),
        Shared::High => band(650, 800),
    }
}

static SHARED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(Shared::Calm as u8);

/// The band [`refresh_shared`] found last (Calm while it is not followed).
pub(crate) fn shared_band() -> Shared {
    match SHARED.load(Ordering::Relaxed) {
        2 => Shared::High,
        1 => Shared::Watch,
        _ => Shared::Calm,
    }
}

/// Whether [`refresh_shared`] follows the band (`VICTORY_SHARED_BAND=1`:
/// on any PC, `=0`: on none): by default on a small RAM host
/// ([`small_ram_host`]) or a card of 12 GiB of budget or less
/// (`crate::host::small_card`), and nowhere else, so that a PC with room
/// keeps everything as before. Nothing acted on that budget before: on a
/// PC of 64 GB the layer uses some 62 % of it, on one of 16 GB (a budget
/// of some 8 GB) it is past it.
pub(crate) fn shared_followed(budget: Option<u64>) -> bool {
    static SET: OnceLock<Option<bool>> = OnceLock::new();
    let set = *SET.get_or_init(|| match std::env::var("VICTORY_SHARED_BAND").as_deref().map(str::trim) {
        Ok("1") => Some(true),
        Ok("0") => Some(false),
        _ => None,
    });
    set.unwrap_or_else(|| small_ram_host() || budget.is_some_and(crate::host::small_card))
}

/// Called by the once-per-second sweep with `Host::video_memory`'s numbers:
/// the band it changed to, if it changed. On entering High the sweep lets
/// go once of what is kept spare (`crate::host` says what); from Watch on
/// less is kept as it comes back ([`short_of_ram`]). Each change is said
/// with both numbers.
pub(crate) fn refresh_shared(memory: Option<[u64; 4]>) -> Option<Shared> {
    let [budget, _, shared_budget, shared_used] = memory?;
    let was = shared_band();
    let band = if shared_followed(Some(budget)) { next_shared(was, shared_used, shared_budget) } else { Shared::Calm };
    if band == was {
        return None;
    }
    SHARED.store(band as u8, Ordering::Relaxed);
    tracing::info!(
        frame = crate::frame::current(),
        "the PC's memory shared with the card: {} MB in use of a budget of {} MB ({:.0} %), from {:?} to {:?} \
         (Watch from 70 %, High from 85 %; VICTORY_SHARED_BAND=0: not followed)",
        shared_used >> 20,
        shared_budget >> 20,
        shared_used as f64 * 100.0 / shared_budget.max(1) as f64,
        was,
        band
    );
    Some(band)
}

/// Whether what comes back for reuse is kept sparingly: on a small RAM host
/// ([`small_ram_host`]), or from Watch on in [`shared_band`].
pub(crate) fn short_of_ram() -> bool {
    small_ram_host() || shared_band() >= Shared::Watch
}

/// Whether [`shared_band`] is High: less is kept for sends and idle host
/// lists (`crate::host`, `crate::list`).
pub(crate) fn shared_high() -> bool {
    shared_band() == Shared::High
}

fn next_pressure(was: bool, physical: u64, commit: u64) -> bool {
    if was {
        physical < 5 * GIB || commit < 6 * GIB
    } else {
        physical <= 3 * GIB || commit <= 4 * GIB
    }
}

/// Called by the once-per-second submission sweep: whether the system has
/// just become short of memory. A failed sample keeps the last good state.
pub(crate) fn refresh() -> bool {
    if !on() {
        return false;
    }
    let Some(status) = memory_status() else { return false };
    let (physical, commit) = (status.ullAvailPhys, status.ullAvailPageFile);
    HEADROOM.store(physical.min(commit), Ordering::Relaxed);
    let was = pressured();
    let pressure = next_pressure(was, physical, commit);
    if pressure != was {
        PRESSURED.store(pressure, Ordering::Relaxed);
        if pressure {
            tracing::warn!(
                pid = std::process::id(),
                frame = crate::frame::current(),
                physical_free_mib = physical >> 20,
                available_commit_mib = commit >> 20,
                buffer_mirror_span_mib = crate::shadow::live_span_bytes() >> 20,
                texture_staging_mib = crate::texture::live_staging_bytes() >> 20,
                spare_scratch_mib = crate::host::ALIVE[4].load(Ordering::Relaxed) >> 20,
                game_filled_gpu_texture_mib = crate::resource::KINDS[1].load(Ordering::Relaxed) >> 20,
                "host memory pressure: limiting texture prefetch and idle upload storage"
            );
        } else {
            tracing::info!(physical_free_mib = physical >> 20, available_commit_mib = commit >> 20, "host memory pressure cleared");
        }
    }
    pressure && !was
}

pub(crate) fn headroom() -> Option<u64> {
    let bytes = HEADROOM.load(Ordering::Relaxed);
    (bytes != u64::MAX).then_some(bytes)
}

pub(crate) fn pressured() -> bool {
    PRESSURED.load(Ordering::Relaxed)
}

/// True when OS commit headroom is critically low (< 1.5 GiB) or an allocation
/// just failed with `E_OUTOFMEMORY`, requiring emergency throttling.
pub(crate) fn critical_commit() -> bool {
    if !on() {
        return false;
    }
    let now = crate::host::coarse_ms();
    now < RETRY_UPLOAD_AT.load(Ordering::Relaxed) || sample_commit(now) < 1536 * MIB
}

/// A soft per-frame admission limit. A single texture can cross it. The
/// queue retains unserved slots so they can be retried on later frames.
pub(crate) fn prefetch_limit(bytes: u64, pressure: bool, critical: bool, limited: bool, staging: u64) -> u64 {
    if critical {
        bytes.min(16 << 20)
    } else if pressure || (limited && staging >= GIB) {
        bytes.min(64 << 20)
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bake_and_lookahead_leave_room_for_live_game_allocations() {
        for (commit, bake, lookahead, growth) in [
            (128 * MIB, false, false, false),
            (256 * MIB, true, false, false),
            (512 * MIB, true, true, false),
            (768 * MIB, true, true, false),
            (1024 * MIB, true, true, true),
        ] {
            assert_eq!(has_upload_reserve(commit, true), bake);
            assert_eq!(has_upload_reserve(commit, false), lookahead);
            assert_eq!(has_growth_reserve(commit, false), growth);
        }
    }

    #[test]
    fn either_constraint_enters_pressure_and_both_must_recover() {
        assert!(next_pressure(false, 12 * GIB, 4 * GIB));
        assert!(next_pressure(false, 3 * GIB, 16 * GIB));
        assert!(next_pressure(true, 4 * GIB, 7 * GIB));
        assert!(next_pressure(true, 6 * GIB, 5 * GIB));
        assert!(!next_pressure(true, 5 * GIB, 6 * GIB));
        assert!(!next_pressure(false, 4 * GIB, 5 * GIB));
    }

    #[test]
    fn prefetch_respects_existing_budgets_and_resumes_after_recovery() {
        assert_eq!(prefetch_limit(512 << 20, false, false, true, GIB), 64 << 20);
        assert_eq!(prefetch_limit(512 << 20, false, false, true, GIB - 1), 512 << 20);
        assert_eq!(prefetch_limit(512 << 20, false, false, false, 2 * GIB), 512 << 20);
        assert_eq!(prefetch_limit(0, true, true, false, 0), 0);
        assert_eq!(prefetch_limit(8 << 20, true, true, false, 0), 8 << 20);
        assert_eq!(prefetch_limit(512 << 20, true, false, false, 0), 64 << 20);
        assert_eq!(prefetch_limit(512 << 20, true, true, false, 0), 16 << 20);
        assert_eq!(prefetch_limit(192 << 20, false, false, false, 0), 192 << 20);
    }

    #[test]
    fn a_small_ram_host_is_one_of_24_gib_or_less() {
        // What a PC of 16 GB and one of 32 GB report.
        assert!(small_ram(15_800 * MIB));
        assert!(small_ram(24 * GIB));
        assert!(!small_ram(24 * GIB + 1));
        assert!(!small_ram(31_900 * MIB));
        assert!(!small_ram(64 * GIB));
    }

    #[test]
    fn shared_band_enters_at_70_and_85_percent() {
        let budget = 8000 * MIB;
        let at = |percent: u64| budget / 100 * percent;
        assert_eq!(next_shared(Shared::Calm, at(62), budget), Shared::Calm);
        assert_eq!(next_shared(Shared::Calm, at(69), budget), Shared::Calm);
        assert_eq!(next_shared(Shared::Calm, at(70), budget), Shared::Watch);
        assert_eq!(next_shared(Shared::Calm, at(84), budget), Shared::Watch);
        assert_eq!(next_shared(Shared::Calm, at(85), budget), Shared::High);
        assert_eq!(next_shared(Shared::Watch, at(85), budget), Shared::High);
        // Past the budget (the system moves memory out to the disk).
        assert_eq!(next_shared(Shared::Calm, 2 * budget, budget), Shared::High);
    }

    #[test]
    fn shared_band_leaves_only_five_points_lower() {
        let budget = 8000 * MIB;
        let at = |percent: u64| budget / 100 * percent;
        assert_eq!(next_shared(Shared::High, at(81), budget), Shared::High);
        assert_eq!(next_shared(Shared::High, at(80), budget), Shared::High);
        assert_eq!(next_shared(Shared::High, at(79), budget), Shared::Watch);
        assert_eq!(next_shared(Shared::High, at(64), budget), Shared::Calm);
        assert_eq!(next_shared(Shared::Watch, at(66), budget), Shared::Watch);
        assert_eq!(next_shared(Shared::Watch, at(65), budget), Shared::Watch);
        assert_eq!(next_shared(Shared::Watch, at(64), budget), Shared::Calm);
        // A use that goes up and down around a line does not toggle.
        let mut band = Shared::Calm;
        for percent in [70, 68, 69, 66, 71, 65] {
            band = next_shared(band, at(percent), budget);
            assert_eq!(band, Shared::Watch, "at {percent} %");
        }
        for percent in [86, 82, 84, 80] {
            band = next_shared(band, at(percent), budget);
            assert_eq!(band, Shared::High, "at {percent} %");
        }
    }

    #[test]
    fn shared_band_without_a_budget_stays_as_it_was() {
        for was in [Shared::Calm, Shared::Watch, Shared::High] {
            assert_eq!(next_shared(was, 5 * GIB, 0), was);
        }
    }
}
