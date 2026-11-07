//! Tier 0 type feedback: record, per dynamic call-site, the runtime `Any` tags
//! observed for the receiver and arguments.
//!
//! This is the dynamic complement to the static `zun prove` analysis: it answers
//! "where does `any` actually persist at run time, and with which shapes?" by
//! counting (not typing) the tags seen on the slow dynamic dispatch path.
//!
//! # Invariants (RTS "count, don't type")
//! - Recording lives **only** on the dynamic `any` path (`col_dynamic_call` and
//!   the dynamic member arms). Typed receivers resolve to `CallBuiltin` / typed
//!   natives and never reach here, so typed/JIT code pays nothing.
//! - Collection is gated by [`enabled`]: off by default, turned on by the
//!   `ZUN_FEEDBACK` environment variable or by [`set_enabled`]. When off, the
//!   hook is a single relaxed atomic load on an already-slow path and returns.
//! - Witnesses are **capped** per histogram ([`WITNESS_CAP`] distinct tags) with
//!   an overflow flag; the running total keeps rising past the cap so a
//!   megamorphic site is never mistaken for its first few witnesses.
//!
//! The side table is thread-local (the interpreter is single-threaded per realm)
//! and never touched by the hot typed path.

use crate::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use zun_module::Module;

/// Distinct tags kept per histogram before the overflow flag trips. Chosen so a
/// mono/bi/polymorphic site is recorded exactly, while a megamorphic site keeps
/// counting into the overflow total rather than growing without bound.
pub const WITNESS_CAP: usize = 8;

/// A broad runtime shape, the unit of frequency counting. Deliberately coarse
/// ("count, don't type"): it names the observed tag, never a formed type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TagClass {
    Int,
    Float,
    Bool,
    String,
    Null,
    Undefined,
    Array,
    Map,
    Set,
    Function,
    Symbol,
    Object,
    Other,
}

impl TagClass {
    pub fn name(self) -> &'static str {
        match self {
            TagClass::Int => "Int",
            TagClass::Float => "Float",
            TagClass::Bool => "Bool",
            TagClass::String => "String",
            TagClass::Null => "Null",
            TagClass::Undefined => "Undefined",
            TagClass::Array => "Array",
            TagClass::Map => "Map",
            TagClass::Set => "Set",
            TagClass::Function => "Function",
            TagClass::Symbol => "Symbol",
            TagClass::Object => "Object",
            TagClass::Other => "Other",
        }
    }
}

/// A capped tag-frequency histogram for one observation slot (receiver or a
/// given argument position).
#[derive(Debug, Default, Clone)]
pub struct Histogram {
    /// First [`WITNESS_CAP`] distinct tags with their counts, in first-seen
    /// order.
    entries: Vec<(TagClass, u64)>,
    /// Set once a tag beyond the cap is seen (the site is megamorphic).
    overflow: bool,
    /// Every observation ever counted here, including those past the cap.
    total: u64,
}

impl Histogram {
    fn observe(&mut self, tag: TagClass) {
        self.total += 1;
        if let Some(entry) = self.entries.iter_mut().find(|(seen, _)| *seen == tag) {
            entry.1 += 1;
        } else if self.entries.len() < WITNESS_CAP {
            self.entries.push((tag, 1));
        } else {
            self.overflow = true;
        }
    }

    /// Distinct witnessed tags with counts, highest count first (ties broken by
    /// tag order for a deterministic dump).
    pub fn sorted(&self) -> Vec<(TagClass, u64)> {
        let mut sorted = self.entries.clone();
        sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        sorted
    }

    pub fn total(&self) -> u64 {
        self.total
    }

    pub fn overflowed(&self) -> bool {
        self.overflow
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// Observed tags at one call-site: the receiver and each argument position.
#[derive(Debug, Default, Clone)]
pub struct SiteFeedback {
    pub receiver: Histogram,
    pub args: Vec<Histogram>,
    /// How many times this site was entered (dynamic calls recorded).
    pub hits: u64,
}

/// A stable identity for a dynamic call-site.
///
/// The key is `(module, method, bytecode offset)`. The method rid and the
/// instruction's byte offset are assigned deterministically by the compiler, so
/// the same source call-site maps to the same key on every run and recompile —
/// it is not a transient per-execution index. The module component (the loaded
/// `Module`'s address) only disambiguates sites from different modules within a
/// single process; the dump filters by the module it is given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SiteKey {
    module: usize,
    pub method_rid: u32,
    pub offset: u32,
}

thread_local! {
    static TABLE: RefCell<HashMap<SiteKey, SiteFeedback>> = RefCell::new(HashMap::new());
}

/// Collection state: 0 = uninitialized, 1 = on, 2 = off. Initialized lazily from
/// the environment on first query so a normal (feedback-off) run pays only a
/// relaxed atomic load, and only on the dynamic path.
static STATE: AtomicU8 = AtomicU8::new(0);

/// Whether feedback collection is active. Off by default; turned on by the
/// `ZUN_FEEDBACK` environment variable (any non-empty value other than `0`) or
/// by [`set_enabled`].
#[inline]
pub fn enabled() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let on = std::env::var_os("ZUN_FEEDBACK")
                .map(|value| !value.is_empty() && value != "0")
                .unwrap_or(false);
            STATE.store(if on { 1 } else { 2 }, Ordering::Relaxed);
            on
        }
    }
}

/// Force collection on or off programmatically (used by `zun prove --runtime`).
pub fn set_enabled(on: bool) {
    STATE.store(if on { 1 } else { 2 }, Ordering::Relaxed);
}

/// Total number of recorded sites across all modules in this thread. Intended
/// for tests asserting that a feedback-off run records nothing.
pub fn site_count() -> usize {
    TABLE.with(|table| table.borrow().len())
}

/// Clear every recorded site. Callers that want a fresh run (the CLI runtime
/// prove, a test) reset first so earlier runs in the same process do not leak in.
pub fn reset() {
    TABLE.with(|table| table.borrow_mut().clear());
}

/// Classify a runtime value into its observed tag, unboxing an `Any` carrier to
/// its concrete shape first. Runs only on the gated dynamic path.
fn classify(value: &Value) -> TagClass {
    match value {
        Value::Int(..) => TagClass::Int,
        Value::Float(..) => TagClass::Float,
        Value::Bool(_) => TagClass::Bool,
        Value::Str(_) => TagClass::String,
        Value::Null => TagClass::Null,
        Value::Void => TagClass::Undefined,
        Value::Function(_) => TagClass::Function,
        Value::Object(obj) => classify_object(*obj),
        Value::Any(any) => {
            if any.as_symbol().is_some() {
                return TagClass::Symbol;
            }
            match zun_gc::with_current_heap(|heap| Value::unbox_any(*any, heap)) {
                // A nested `Any` cannot be classified further: count it as Other.
                Some(Value::Any(_)) | None => TagClass::Other,
                Some(inner) => classify(&inner),
            }
        }
    }
}

fn classify_object(obj: zun_gc::Obj) -> TagClass {
    zun_gc::with_current_heap(|heap| {
        if heap.array_len(obj).is_some() {
            TagClass::Array
        } else if heap.map_len(obj).is_some() {
            TagClass::Map
        } else if heap.set_len(obj).is_some() {
            TagClass::Set
        } else {
            TagClass::Object
        }
    })
}

/// Record one dynamic call observation at `(module, method_rid, offset)`.
///
/// `call_args` is the raw `dynamicCallN` argument vector: slot 0 is the
/// receiver, slot 1 the member-name string, and slots `2..` the actual call
/// arguments. The caller has already checked [`enabled`]; this does the counting.
pub fn record(module: &Module, method_rid: u32, offset: u32, call_args: &[Value]) {
    let key = SiteKey {
        module: module as *const Module as usize,
        method_rid,
        offset,
    };
    let receiver_tag = call_args.first().map(classify);
    let arg_tags: Vec<TagClass> = call_args.iter().skip(2).map(classify).collect();
    TABLE.with(|table| {
        let mut table = table.borrow_mut();
        let site = table.entry(key).or_default();
        site.hits += 1;
        if let Some(tag) = receiver_tag {
            site.receiver.observe(tag);
        }
        if site.args.len() < arg_tags.len() {
            site.args.resize(arg_tags.len(), Histogram::default());
        }
        for (slot, tag) in arg_tags.into_iter().enumerate() {
            site.args[slot].observe(tag);
        }
    });
}

/// A rendered view of one site, with its source location resolved against the
/// module it belongs to.
#[derive(Debug)]
pub struct SiteReport {
    pub method_rid: u32,
    pub offset: u32,
    pub method_name: String,
    pub source_span: Option<(u32, u32)>,
    pub feedback: SiteFeedback,
}

/// Snapshot the recorded sites that belong to `module`, resolving each site's
/// method name and source span. Sorted by `(method_rid, offset)` for a stable,
/// deterministic order.
pub fn snapshot(module: &Module) -> Vec<SiteReport> {
    let module_id = module as *const Module as usize;
    let mut reports: Vec<SiteReport> = TABLE.with(|table| {
        table
            .borrow()
            .iter()
            .filter(|(key, _)| key.module == module_id)
            .map(|(key, feedback)| {
                let method_name = module
                    .tables
                    .method_defs
                    .get((key.method_rid as usize).wrapping_sub(1))
                    .map(|method| module.strings.get(method.name).to_string())
                    .unwrap_or_default();
                SiteReport {
                    method_rid: key.method_rid,
                    offset: key.offset,
                    method_name,
                    source_span: module.source_span_at(key.method_rid, key.offset),
                    feedback: feedback.clone(),
                }
            })
            .collect()
    });
    reports.sort_by(|a, b| {
        a.method_rid
            .cmp(&b.method_rid)
            .then(a.offset.cmp(&b.offset))
    });
    reports
}

/// Render the recorded sites for `module` as a deterministic text report — the
/// dynamic "where any persists" metric that complements static `zun prove`.
pub fn render(module: &Module) -> String {
    use std::fmt::Write;
    let reports = snapshot(module);
    let mut out = String::new();
    out.push_str("runtime type feedback (dynamic any call-sites)\n");
    if reports.is_empty() {
        out.push_str("  (no dynamic call-sites recorded)\n");
        return out;
    }
    let total_hits: u64 = reports.iter().map(|r| r.feedback.hits).sum();
    let _ = writeln!(
        out,
        "  {} site(s), {} dynamic call(s) observed",
        reports.len(),
        total_hits
    );
    for report in &reports {
        let span = match report.source_span {
            Some((start, end)) => format!("src {start}..{end}"),
            None => "src ?".to_string(),
        };
        let _ = writeln!(
            out,
            "  site m{}@{} ({}, {}) hits={}",
            report.method_rid, report.offset, report.method_name, span, report.feedback.hits
        );
        out.push_str("    receiver: ");
        push_histogram(&mut out, &report.feedback.receiver);
        for (slot, hist) in report.feedback.args.iter().enumerate() {
            if hist.is_empty() {
                continue;
            }
            let _ = write!(out, "    arg{slot}: ");
            push_histogram(&mut out, hist);
        }
    }
    out
}

fn push_histogram(out: &mut String, hist: &Histogram) {
    use std::fmt::Write;
    if hist.is_empty() {
        out.push_str("(none)\n");
        return;
    }
    let parts: Vec<String> = hist
        .sorted()
        .into_iter()
        .map(|(tag, count)| format!("{}={}", tag.name(), count))
        .collect();
    let _ = write!(out, "{}", parts.join(", "));
    if hist.overflowed() {
        let _ = write!(out, ", +overflow(total={})", hist.total());
    }
    out.push('\n');
}
