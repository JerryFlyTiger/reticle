# M146 -- dev/mutate.py survives being killed (journal + recovery) and checks
# the whole list before running (preflight). Entries J/P/B were designed from
# the cold reviewer's checklist plus the fix round; executed by the main
# conversation.
#
# Every entry is observed through `dev_mutate_tests_pass`, which runs
# `python3 dev/test_mutate.py` as one Rust test -- so a FAIL here says "the
# suite went red", not which Python test did. The Python test each entry is
# aimed at is named in the label; the runner's log shows the failing name.
#
# The runner mutates its own source file here. That is safe: the running
# process has already compiled mutate.py, and the mutated copy on disk is what
# test_mutate.py's subprocesses execute.
#
#     python3 dev/mutate.py --config dev/mutations/m146.py

PACKAGE = "core"
TEST_TARGET = "dev_tools_tests"

_M = "dev/mutate.py"


def _e(label, old, new):
    return {"label": label, "file": _M, "old": old, "new": new,
            "test": "dev_mutate_tests_pass"}


MUTATIONS = [
    # ---- J: journal and recovery -----------------------------------------
    _e("J1 DELETION recovery never runs (kill_mid_entry, user_edit, live_pid, touches_pristine)",
       "        journal = load_journal()\n        if journal is not None:",
       "        journal = None\n        if journal is not None:"),
    _e("J2 live pid not refused (live_pid_refuses)",
       "    if pid is not None and is_mutate_process_alive(pid):",
       "    if False and pid is not None and is_mutate_process_alive(pid):"),
    _e("J3 already-pristine fast path removed (recover_touches_already_pristine_file)",
       "        if already_pristine:",
       "        if False:"),
    _e("J4 mid-mutation match never restores (kill_mid_entry_then_next_run_restores)",
       '            if pristine.count(active["old"]) == 1 and cur == pristine.replace(',
       '            if False and pristine.count(active["old"]) == 1 and cur == pristine.replace('),
    _e("J5 unreadable backup raises again (recover_with_missing_backup_does_not_crash)",
       "            except (OSError, UnicodeDecodeError) as e:",
       "            except (UnicodeDecodeError,) as e:"),
    _e("J6 journal deleted even when a file is unrecovered (restore_failure_keeps_journal)",
       "        if unrecovered:",
       "        if False:"),
    _e("J7 journal never cleaned up on a normal run (normal_run_leaves_no_journal)",
       "        else:\n            shutil.rmtree(mutate_dir(), ignore_errors=True)",
       "        else:\n            pass"),
    _e("J8 active entry never journaled before mutating (kill_mid_entry_then_next_run_restores)",
       "                set_active(m)\n",
       "                pass\n"),
    # J9 pins the computation the whole "keep the journal" fix rests on. J6/J7
    # only mutate the control flow around it; weakening the check itself would
    # reintroduce the original bug through a different line (2026-09-18 cold
    # review, finding 2).
    _e("J9 pristine check drops the content comparison (restore_failure_keeps_journal)",
       '                ok = os.path.exists(p) and sha256_of(p) == info["pristine_sha256"]',
       "                ok = os.path.exists(p)"),
    _e("J10 non-UTF-8 backup or file raises again (recover_with_non_utf8_*)",
       "            except (OSError, UnicodeDecodeError) as e:",
       "            except OSError as e:"),
    _e("J11 non-UTF-8 journal not recognised as corrupt (recover_only_with_non_utf8_journal_refuses)",
       "    except (json.JSONDecodeError, OSError, UnicodeDecodeError) as e:",
       "    except (json.JSONDecodeError, OSError) as e:"),
    # J12 restores the behaviour the fourth cold-read round reproduced: an
    # unreadable journal treated as "no journal", so a later unrelated run
    # deletes the backups of a file still sitting mutated.
    _e("J12 DELETION corrupt journal silently discarded again (all three corrupt_journal tests)",
       "    except (json.JSONDecodeError, OSError, UnicodeDecodeError) as e:\n",
       "    except (json.JSONDecodeError, OSError, UnicodeDecodeError) as e:\n        return None\n"),
    # J13-J17 cover the shape validator added after the fifth cold-read round.
    # J13 removes its whole effect; the rest knock out one check each.
    _e("J13 DELETION journal shape never validated (the journal-shape tests)",
       "    problems = validate_journal_shape(data)",
       "    problems = []"),
    _e("J14 missing/non-dict 'files' accepted (journal_empty_dict_*)",
       "    if not isinstance(files, dict):",
       "    if False:"),
    _e("J15 non-integer 'pid' accepted (journal_pid_as_string)",
       "    if pid is not None and (not isinstance(pid, int) or isinstance(pid, bool)):",
       "    if False:"),
    _e("J16 non-dict 'active' accepted (journal_active_not_a_dict)",
       "        if not isinstance(active, dict):",
       "        if False:"),
    _e("J17 file entry without a 'backup' path accepted (journal_file_entry_missing_backup)",
       '            if not isinstance(info.get("backup"), str):',
       "            if False:"),
    _e("J18 file entry that is not an object accepted (journal_file_entry_not_a_dict)",
       "            if not isinstance(info, dict):",
       "            if False:"),
    _e("J19 active missing-key check disabled (journal_active_missing_one_key)",
       '            missing = [k for k in ("label", "file", "old", "new") if k not in active]',
       "            missing = []"),
    _e("J20 active values not required to be strings (journal_active_non_string_old_new)",
       '                non_str = [k for k in ("label", "file", "old", "new") if not isinstance(active[k], str)]',
       "                non_str = []"),
    _e("J21 DELETION orphaned backups deleted again (journal_empty_files_dict_does_not_delete_real_backups)",
       "        orphaned = sorted(on_disk - referenced)",
       "        orphaned = []"),
    _e("J22 JSON boolean accepted as a pid (journal_pid_true_refuses)",
       "    if pid is not None and (not isinstance(pid, int) or isinstance(pid, bool)):",
       "    if pid is not None and not isinstance(pid, int):"),
    _e("J23 absurd pid not bounded (journal_pid_absurdly_large)",
       "        problems.append(f\"'pid' is out of range for a process id (got {pid})\")",
       "        pass"),
    _e("J24 unreadable/directory path crashes the pristine check (recover_with_directory_where_file_expected)",
       "        try:\n            already_pristine = sha256_of(path) == info.get(\"pristine_sha256\")\n        except OSError as e:",
       "        try:\n            already_pristine = sha256_of(path) == info.get(\"pristine_sha256\")\n        except KeyError as e:"),
    _e("J25 pid bound back to the 64-bit range (journal_pid_just_over_32bit)",
       "    elif pid is not None and not (-(2**31) <= pid <= 2**31 - 1):",
       "    elif pid is not None and not (-sys.maxsize - 1 <= pid <= sys.maxsize):"),
    _e("J26 unreadable backup_root crashes the orphan scan (recover_with_unreadable_backup_root)",
       "            names = os.listdir(backup_root())\n        except OSError as e:",
       "            names = os.listdir(backup_root())\n        except KeyError as e:"),
    _e("J27 DELETION startup safety net catches nothing (startup_recovery_unexpected_exception)",
       "    except Exception as e:\n        die(2, f\"mutate.py: unexpected error during startup recovery \"",
       "    except KeyError as e:\n        die(2, f\"mutate.py: unexpected error during startup recovery \""),
    # ---- B: backup names -------------------------------------------------
    _e("B1 backup names drop the index prefix (collision-free naming)",
       "    return f\"{index:04d}__{relpath.replace('/', '__')}\"",
       "    return f\"{relpath.replace('/', '__')}\""),
    # ---- P: preflight ----------------------------------------------------
    _e("P1 DELETION preflight finds nothing (reports_all_problems, unresolvable, mod_qualified)",
       "    problems = preflight(mutations, package, target)",
       "    problems = []"),
    _e("P2 old==new not checked (reports_all_problems_together)",
       '        if m["old"] == m["new"]:',
       '        if False and m["old"] == m["new"]:'),
    _e("P3 old occurrence count not checked (reports_all_problems_together)",
       "        if hits != 1:\n            problems.append(f\"{label}: old occurs",
       "        if False:\n            problems.append(f\"{label}: old occurs"),
    _e("P4 mod names and mod::fn paths not accepted (mod_qualified_test_names)",
       "    names = set(fn_names) | set(mod_names)\n    for mod_name in mod_names:",
       "    names = set(fn_names)\n    for mod_name in []:"),
    _e("P5 lib target not resolved to src/ (preflight_lib_and_substring_ok)",
       '    if target == "lib":\n        src_dir = os.path.join(crate_dir, "src")',
       '    if target == "lib-never":\n        src_dir = os.path.join(crate_dir, "src")'),
    _e("P6 misspelt test substring accepted (mod_qualified ... misspelt_rejected)",
       "                names |= candidate_test_names(",
       "                names |= {m['test']} | candidate_test_names("),
]
