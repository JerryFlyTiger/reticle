# M52 mutation list (GC mark traversal of Values held inside Value::Ext).
#
# List designed by the reviewer, executed by the main conversation. Kept as a
# regression list: after changing the mark logic in gc.rs, the trace field of
# ExtRef, or any tracer, run
# `dev/mutate.py --config dev/mutations/m52.py` to confirm these guards are
# still being watched by tests.
# If an entry turns into SKIP, the code has drifted from the original string in
# the list; update it or delete the entry.
#
# Two things to state up front so they don't need re-deriving next time:
#
# * The observable consequence of M2 (removing the visited dedup gate) is a
#   **hang**, not an assertion failure. A pure Ext<->Ext cycle sends mark into
#   an infinite loop and `cargo test` never returns. So it declares
#   `expect: "hang"` with a short timeout. Do not change it to expect FAIL.
#
# * The negative control `genuinely_dead_cycle_still_collected_alongside_ext_tracers`
#   is **not listed here**, because no single-line change in this diff can turn
#   it red. What it guards against is a future regression where the tracer
#   makes mark overly conservative and treats Ext as a conservative root;
#   the current code does not have that behavior. Honestly recorded as having
#   no observable point, rather than hard-coding an entry just to pad the count.
#
# Likewise, changing `ExtRef::new` from 2 parameters to 3 has no test-level
# observable point either -- its enforcement is at the type checker: omitting
# the argument is a compile failure, not a test turning red.

PACKAGE = "core"
TEST_TARGET = "gc_ext_root_tests"

GC = "crates/elisp/src/gc.rs"
KEYMAP = "crates/core/src/keymap.rs"
EDITOR = "crates/core/src/editor.rs"
UI = "crates/core/src/builtins/ui.rs"

MUTATIONS = [
    {
        "label": "M1 mark whole Ext tracer dispatch (reverts to pre-fix behavior)",
        "file": GC,
        "old": """                    if let Some(trace) = e.trace {""",
        "new": """                    if let Some(trace) = Option::<crate::value::ExtTracer>::None {""",
        "test": "nested_prefix_keymap_closure_survives_gc",
    },
    {
        "label": "M2 visited dedup gate (pure Ext cycle loops forever)",
        "file": GC,
        "old": """                        if visited.insert(Rc::as_ptr(&e.obj) as *const () as usize) {""",
        "new": """                        {""",
        "test": "mutually_referential_keymaps_do_not_hang_gc",
        "expect": "hang",
        # The hang case doesn't need to wait out the default 600s; normal runs finish in under 1s.
        "timeout": 90,
    },
    {
        "label": "M3 Keymap tracer (nested prefix keymap path)",
        "file": KEYMAP,
        "old": """        Some(trace_keymap),""",
        "new": """        None,""",
        "test": "nested_prefix_keymap_closure_survives_gc",
    },
    {
        "label": "M4 Keymap tracer (keymap reachable only via symbol value)",
        "file": KEYMAP,
        "old": """        Some(trace_keymap),""",
        "new": """        None,""",
        # This one specifically blocks the half-fix that only recurses into
        # editor-held keymaps in the root provider: that kind of fix makes M3
        # pass while this one turns red.
        "test": "keymap_reachable_only_via_global_variable_survives_gc",
    },
    {
        "label": "M5 Keymap tracer (buffer-local map / local-set-key)",
        "file": KEYMAP,
        "old": """        Some(trace_keymap),""",
        "new": """        None,""",
        "test": "local_set_key_closure_survives_gc",
    },
    {
        "label": "M6 OverlayData tracer (make-overlay construction site)",
        "file": UI,
        "old": """        bb.insert_overlay(ov.clone());
        Ok(Value::Ext(ExtRef {
            tag: OVERLAY_TAG,
            obj: ov,
            trace: Some(trace_overlay),
        }))""",
        "new": """        bb.insert_overlay(ov.clone());
        Ok(Value::Ext(ExtRef {
            tag: OVERLAY_TAG,
            obj: ov,
            trace: None,
        }))""",
        "test": "deleted_overlay_props_survive_gc",
    },
    {
        "label": "M8 OverlayData tracer (overlays-in construction site)",
        "file": UI,
        "old": """                Value::Ext(ExtRef {
                    tag: OVERLAY_TAG,
                    obj: ov,
                    trace: Some(trace_overlay),
                })""",
        "new": """                Value::Ext(ExtRef {
                    tag: OVERLAY_TAG,
                    obj: ov,
                    trace: None,
                })""",
        "test": "overlays_in_result_props_survive_gc",
    },
    {
        "label": "M9 trace_buffer traverses buffer overlays",
        "file": EDITOR,
        "old": """    for ov in &b.overlays {
        let Ok(o) = ov.try_borrow() else {
            return false;
        };
        walk_overlay_props(&o, sink);
    }""",
        "new": """""",
        "test": "killed_buffer_overlay_props_survive_gc",
    },
    {
        "label": "M7 Buffer tracer (buffer killed but still held by an elisp value)",
        "file": EDITOR,
        "old": """            trace: Some(trace_buffer),""",
        "new": """            trace: None,""",
        "test": "killed_but_referenced_buffer_locals_survive_gc",
    },
]
