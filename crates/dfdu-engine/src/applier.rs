//! The applier core: gate, chunk and close handling, pending accounting, jobs, cuts, rescans,
//! stalls, quiescence and hard links. No threads: the runtime and the simulation harness both
//! drive it. One `Applier`, its `impl` split by concern over the submodules; this module holds
//! the state, the commands and their dispatch.

mod command;
mod delete;
mod estimate;
mod guards;
mod invariants;
mod jobs;
mod links;
mod measure;
mod messages;
mod model;
mod native;
mod rescan;
mod resume;
mod scanner;
#[cfg(test)]
mod test_util;
mod undo;
mod work;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::Arc;

use dfdu_scan::{Backend, DirId, JobId, JobOptions, ListingStatus, Shared};
use dfdu_tree::{Class, Ix, Kind, Meta, NodeRef, Pages, Tree};

pub use command::Command;
pub use measure::MEASURE_MIN;
pub use model::{
    Count, DeleteProgress, DeleteSummary, Estimate, JobInfo, JobKind, JobPhase, MeasureProgress,
    Model, NATIVE_LINES, NativeRun, PROBLEMS_KEPT, StalledInfo, Stats, Status, Timing, UndoInfo,
};
pub use native::native_argv;
pub(crate) use native::native_req;

use crate::classifier::CheckReq;
use crate::extents::ExtentTable;
use links::{LinkTable, Scope};
use resume::Kept;
pub use resume::unrefreshed;
use work::Work;

/// A `Close` that arrived before some of its directory's chunks: their number, the status, and
/// the classes it carries.
type HeldClose = (u32, (ListingStatus, Vec<(DirId, Class)>));

struct Job {
    id: JobId,
    kind: JobKind,
    target: NodeRef,
    root: NodeRef,
    scope: Scope,
    opts: JobOptions,
    map: Pages<Slot>,
    held_close: HashMap<u32, HeldClose>,
    stalled: HashMap<u32, u64>,
    submitted_seq: u64,
    phase: JobPhase,
    /// Directories it scanned were queued for extent measurement as their listings closed.
    measures: bool,
    /// A resume's rescan: the loaded directories it keeps.
    kept: Option<Box<Kept>>,
}

/// What the applier knows about a `DirId` of a job.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Slot {
    /// Not named by any applied chunk yet.
    Unknown,
    Live(NodeRef),
    /// Named by a chunk that was discarded (its directory was cut, deleted or abandoned), so
    /// it has no node and never will; its messages are discarded.
    Dead,
}

impl Job {
    /// A rescan that has not swapped in yet: it still blocks overlapping rescans and deletes,
    /// including while it settles hard links after its scan finished.
    fn swap_pending(&self) -> bool {
        matches!(self.phase, JobPhase::Running | JobPhase::Finishing)
    }

    fn slot(&self, d: DirId) -> Slot {
        self.map.get(d.0 as usize).copied().unwrap_or(Slot::Unknown)
    }
    /// The node of `d`, when it has one.
    fn live(&self, d: DirId) -> Option<NodeRef> {
        match self.slot(d) {
            Slot::Live(r) => Some(r),
            _ => None,
        }
    }
    fn put(&mut self, d: DirId, s: Slot) {
        let i = d.0 as usize;
        if self.map.len() <= i {
            self.map.grow_to(i + 1, Slot::Unknown);
        }
        self.map[i] = s;
    }
    fn set_live(&mut self, d: DirId, r: NodeRef) {
        self.put(d, Slot::Live(r));
    }
}

/// What an applier is set to for its session, fixed when it starts. The defaults are a local
/// scan that measures every file, with no classifier pool, run as an ordinary user.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent settings of the session, not the states of one machine"
)]
pub struct ApplierSettings {
    /// Set for a remote scan: the agent that runs its jobs.
    pub(crate) remote: Option<Arc<crate::remote::Link>>,
    /// Refuse every delete request and native command (dry runs aside).
    pub read_only: bool,
    /// The tree was loaded from a file: refuse everything that would touch the filesystem.
    pub(crate) imported: bool,
    /// Measure extents: each directory of a scan as its listing closes, and what is left of a
    /// job's tree when it ends.
    pub measure: bool,
    /// Files allocating less are not measured (`MEASURE_MIN` in the engine).
    pub measure_min: u64,
    /// Whoever deletes runs as root (the agent, for a remote engine), so read-only btrfs
    /// snapshots can be deleted.
    pub privileged: bool,
    /// A classifier pool answers guard checks; without one none are queued.
    pub checks: bool,
}

impl Default for ApplierSettings {
    fn default() -> Self {
        ApplierSettings {
            remote: None,
            read_only: false,
            imported: false,
            measure: true,
            measure_min: 1,
            privileged: false,
            checks: false,
        }
    }
}

pub struct Applier<B: Backend> {
    /// The local scanner. Its backend serves local reads; its jobs run only when
    /// `settings.remote` is unset, and are steered through the `scanner_*` methods, which pick
    /// one of the two.
    scan: Arc<Shared<B>>,
    pub settings: ApplierSettings,
    jobs: BTreeMap<u32, Job>,
    /// Options of finished jobs dropped from `jobs`, for rescans of what they scanned.
    retired: HashMap<u32, JobOptions>,
    /// Mounts the user entered this session ([`Command::Enter`]), by the device and inode of
    /// their roots: every rescan enters them again.
    entered: HashMap<NodeRef, Vec<(u64, u64)>>,
    next_job: u32,
    links: LinkTable,
    work: VecDeque<Work>,
    retry_abandon: Vec<(JobId, DirId)>,
    /// Delete plans queued and running, and what of them can still be put back.
    delete: delete::Deleting,
    /// Paths whose directories are rescanned when the running undo or native command ends:
    /// what it put back or what it cleaned.
    rescan_paths: Vec<dfdu_tree::HostPath>,
    /// What is kept of resumes beside their walks.
    resume: resume::Resumes,
    /// Rescans waiting for their parent's listing to close, and whether each enters a mount:
    /// a directory's class can depend on its siblings, which a listing still being read does
    /// not have yet.
    rescans_after_listing: Vec<(NodeRef, bool)>,
    extents: ExtentTable,
    /// Devices whose filesystem can share extents, from the scanner's probes.
    extent_devs: HashSet<u64>,
    /// Where the extent measurement walk is and what it has in flight.
    measure: measure::Measuring,
    /// Requests for the classifier pool, taken by the runtime after each slice
    /// (`settings.checks`).
    outgoing_checks: Vec<CheckReq>,
}

pub(crate) fn meta_of(tree: &Tree, ix: Ix) -> Meta {
    let (apparent, alloc) = tree.raw_size(ix);
    let (dev, ino) = tree.dev_ino(ix);
    Meta {
        kind: tree.kind(ix),
        apparent,
        alloc,
        mtime: tree.mtime(ix),
        dev,
        ino,
        nlink: 1,
        mount: 0,
    }
}

fn empty_dir_meta() -> Meta {
    Meta {
        kind: Kind::Dir,
        apparent: 0,
        alloc: 0,
        mtime: 0,
        dev: 0,
        ino: 0,
        nlink: 1,
        mount: 0,
    }
}

impl<B: Backend> Applier<B> {
    pub fn new(scan: Arc<Shared<B>>, settings: ApplierSettings) -> Self {
        Applier {
            scan,
            settings,
            jobs: BTreeMap::new(),
            retired: HashMap::new(),
            entered: HashMap::new(),
            next_job: 1,
            links: LinkTable::default(),
            work: VecDeque::new(),
            retry_abandon: Vec::new(),
            delete: delete::Deleting::default(),
            rescan_paths: Vec::new(),
            rescans_after_listing: Vec::new(),
            resume: resume::Resumes::default(),
            extents: ExtentTable::default(),
            extent_devs: HashSet::new(),
            measure: measure::Measuring::default(),
            outgoing_checks: Vec::new(),
        }
    }

    #[must_use]
    pub fn has_work(&self) -> bool {
        !self.work.is_empty() || self.measure_ready()
    }

    /// Roots of running rescan shadows, for the structural invariant check.
    #[must_use]
    pub fn shadow_roots(&self, tree: &Tree) -> Vec<Ix> {
        self.jobs
            .values()
            .filter(|j| j.kind == JobKind::Rescan && j.phase != JobPhase::Done)
            .filter_map(|j| tree.resolve(j.root))
            .filter(|&ix| !tree.is_visible(ix))
            .collect()
    }

    pub fn refresh_status(&self, m: &mut Model) {
        m.status.jobs = self
            .jobs
            .values()
            .map(|j| JobInfo {
                id: j.id,
                kind: j.kind,
                target: j.target,
                phase: j.phase,
            })
            .collect();
        let mut stalled = Vec::new();
        for j in self.jobs.values().filter(|j| j.phase == JobPhase::Running) {
            for (&d, &since) in &j.stalled {
                if let Some(r) = j.live(DirId(d)) {
                    stalled.push(StalledInfo {
                        node: r,
                        since_ns: since,
                    });
                }
            }
        }
        m.status.stalled = stalled;
        m.status.settling = !self.work.is_empty() || self.resume.waiting.is_some();
        if self.measure_idle() {
            m.status.measuring = None;
        }
    }
}
