#!/usr/bin/env python3
"""Send requests to a real LSP server and see what it *actually* replies.

This project's rule is: probe a real server before touching anything (see
PLAN.md's M46/M48 records). Rewriting the probe every time creates friction
that tempts you to skip that step, so it's a fixed tool instead.

Two wrong calls it has caught (both in M48, both would have directly decided
the milestone's scope):

1. Capabilities declared by `initialize` are **untrustworthy in both
   directions**. verible declares `hoverProvider: false` yet returns a full
   hover; it declares `referencesProvider: true` yet returns incomplete
   results. So this tool always prints capabilities, but also always sends
   a real request anyway.

2. **When a probe returns "unsupported", suspect your own parameters before
   the server.** The first rename probe returned `{"changes": null}` and
   was almost judged unimplemented — the actual cause was the position
   being off by one column (char 14 pointed at `;`; the symbol is at
   char 13). Because of this, `--line/--char` prints the character at that
   position along with its surrounding context, so off-by-one errors are
   visible right away.

M54 added a third lesson. It's not at the protocol layer but the
**transport layer**, and it also nearly caused a misjudgment:

3. **`read()` blocks forever, and `request()` silently returns `None` once
   its budget is used up.** Probing rust-analyzer for completion, the first
   version returned "0 items"; the second version just hung for two
   minutes. Neither was because it's unsupported — it needs to finish over
   a dozen `$/progress` indexing stages first (Fetching /
   Building CrateGraph / cachePriming / flycheck), and along the way it
   also sends a **server-to-client request**
   (`window/workDoneProgress/create`); without a reply it won't proceed.
   Only `pump()` (non-blocking select polling + auto-replying to server
   requests) could actually get an answer: the same position returned 16
   items once indexing had finished. **This is the same class of error as
   lesson 2, just moved to the transport layer — suspect your own probe
   before the server.**

M57 added a fourth lesson, completing the other half of lesson 3:

4. **"The server accepts a request but never replies" is a real failure
   mode, and the probe previously couldn't see it.** `slang-server` 0.2.9
   has no `documentFormattingProvider`; sending it
   `textDocument/formatting` anyway got **no result, no error, nothing at
   all**. The old generic `--method` path used the blocking `request()`,
   so the probe itself **hung there forever** (confirmed in practice — it
   had to be killed externally), looking like the probe was broken rather
   than the server staying silent. The generic path now uses
   `request_pumped` + `--timeout`, and prints `TIMEOUT` explicitly on
   timeout.
   The same round also fixed another dimension: the formatting family
   **has no position**, and the old version hard-coded "must have
   `--line/--char`" into the shared path, meaning these two methods
   couldn't be sent at all — **this is the third time this tool has
   produced or nearly produced a false conclusion by missing a dimension
   to ask about** (the first two were M55's file visibility ->
   `--also-open`, and M56/slang's rootUri -> `--root`); the difference this
   time is the missing dimension is at the transport layer. **"No
   response" and "got back an empty result" are different answers — don't
   let the tool display the former as the latter, or as a crash.**

Lesson five was added after M59; it belongs to the same family as lessons
3/4 (**missing a dimension to ask about, yet again**), just this time the
dimension is the shape of `params`:

5. **Not every method has a `textDocument`.** The shared path always
   started from `params = {"textDocument": {"uri": uri}}` as the base for
   every request, so a workspace-level method like `workspace/symbol`
   **couldn't be sent at all** — it takes `{"query": "..."}`, and an extra
   `textDocument` field is not the shape in the spec. This is the same
   error as M57's discovery that "the formatting family has no position",
   just one level further out: that time the missing thing was position,
   this time it's the `textDocument` field itself. **Before adding a new
   method, go check the spec for what its params actually look like — don't
   assume it's a variant of an existing shape.**
   (The file still needs a didOpen: verible needs an open file present
   before it will load the project.)

   **This bit again four more times on 2026-08-13**, while backfilling
   coverage for the unconsumed capabilities of slang-server: `documentLink`
   only takes textDocument (the generic path hard-required
   `--line/--char`, so it simply couldn't be sent); `inlayHint` takes a
   **range, not a position** (probing with a single point would
   misread "this span has no hint" as "the server refuses to give one");
   `callHierarchy/incomingCalls` takes `{"item": ...}`, **it doesn't even
   have textDocument** — you first need `prepareCallHierarchy` to get an
   item out, making it a **two-step** flow; `workspace/executeCommand`
   takes `{command, arguments}`, while the old version treated every
   `workspace/*` method as `{query}`. Three of these four would have the
   old version silently send the wrong shape — the response you got back
   would look exactly like "unsupported". Each of them now has its own
   dedicated path (`--call-hierarchy`, `--command`) or branch.

Lesson six was added on 2026-08-13, and it's worse than the previous five —
the first five let the probe **fail to ask** something; this one makes it
**give the wrong answer**, and the direction of the error is "looks like
the server doesn't support it":

6. **`select` can't see bytes sitting in Python's own buffer.** `pump` uses
   `select.select([self.pout], ...)` to decide whether there's anything to
   read, but `self.pout` defaults to a buffered `BufferedReader`: a single
   `readline` pulls a whole chunk from the fd into the buffer, and several
   later messages end up **sitting in that buffer while the fd has
   nothing**. `select` then reports "no data" forever, `pump` spins until
   timeout, while the response had actually arrived long ago. Confirmed in
   practice: `textDocument/documentLink` timed out on its own (even at 45
   seconds), but replied fine when run inside `--sweep` — the only
   difference was that the ten requests ahead of it in the sweep had
   already stirred the buffer. The fix is `bufsize=0`, making the fd the
   single source of truth (at the cost of having to do our own
   `_read_exactly`, since a raw read can come back short).
   **The cost has already been paid once**: this bug got
   `workspace/symbol` recorded as "unsupported by slang too" (commit
   `9d906ad`), and once fixed, it **answered just fine**. Anything this
   tool ever reported TIMEOUT for, with a conclusion written into
   PLAN.md, must be re-tested with the fixed version before it counts.
   (Re-tested: `textDocument/formatting` against slang-server genuinely
   still TIMEOUTs; M57's conclusion is unaffected.)

Lesson seven was added on 2026-08-14 (M63). It belongs to the same family
as lesson 5 ("missing a dimension to ask about"), but this time the
dimension shifted to the **observation side** — the previous misses were
all in "how to ask", this one is in "what you can even see":

7. **Only being able to see the primary file's diagnostics means "sent but
   dropped" gets displayed as "never sent".**
   `await_diagnostics(uri)` filters by uri, and non-matching messages are
   **read and thrown away**. M63 needed to answer "if a second file is
   opened with didOpen on the same connection, will the server also send
   diagnostics for it" — under the old path this question **couldn't be
   answered**, and the output looked like a clear "no". Added
   `--diagnostics-all` (paired with `--also-open`); confirmed in practice
   that verible sends one notification for each of three didOpen'd files —
   0 items for a clean file, 3 items for a deliberately broken one. **This
   decided what M63 could claim**: not just "the command is no longer
   silent" but that it immediately surfaces the syntax error.
   The same round also **replayed lesson 3 verbatim** inside a new
   function: the first version was written as `for _ in range(15):
   self.read()`; verible went quiet after sending three notifications, and
   `read` blocked forever, hanging the probe until it was killed
   externally. **Whenever "we don't know how many messages are coming" is
   true, never use "read at most N messages" as the cap** — use select
   plus a time budget instead. Writing the lesson in the header doesn't
   stop it from being made again in new code.

Usage
    # only look at declared capabilities
    dev/lsp-probe.py --file foo.sv

    # workspace-level (no position, takes query; an empty string is legal
    # and means "everything")
    dev/lsp-probe.py --file foo.sv --method workspace/symbol --query alu

    # look at lint diagnostics (wait for publishDiagnostics)
    dev/lsp-probe.py --file foo.sv --diagnostics

    # send a request at a given position (0-based line/char, matching LSP)
    dev/lsp-probe.py --file foo.sv --method textDocument/documentHighlight \\
                     --line 2 --char 9

    # codeAction needs a diagnostic as context; pick the Nth one with --at-diagnostic
    dev/lsp-probe.py --file foo.sv --method textDocument/codeAction --at-diagnostic 0

    # rename needs a new name
    dev/lsp-probe.py --file foo.sv --method textDocument/rename \\
                     --line 1 --char 13 --new-name sig_renamed

    # horizontal sweep: send ten methods once each, and see at a glance
    # which cells are inconsistent between "declared" and "actual"
    dev/lsp-probe.py --server slang-server --root demo/rtl \\
                     --file demo/rtl/core/alu.sv --sweep --line 32 --char 9

    # formatting has no position (whole document); on timeout prints
    # TIMEOUT instead of hanging
    dev/lsp-probe.py --server slang-server --root demo/rtl \\
                     --file demo/rtl/core/alu.sv --method textDocument/formatting

    # rangeFormatting uses the whole line pointed at by --line as its range
    dev/lsp-probe.py --file foo.sv --method textDocument/rangeFormatting --line 32

    # inlayHint takes a range (automatically the whole file); documentLink
    # only takes textDocument
    dev/lsp-probe.py --file foo.sv --method textDocument/inlayHint
    dev/lsp-probe.py --file foo.sv --method textDocument/documentLink

    # callHierarchy is two-step: prepare gets the item, then use the item
    # to ask incoming/outgoing
    dev/lsp-probe.py --server slang-server --root demo/rtl \\
                     --file demo/rtl/core/alu.sv --call-hierarchy --line 5 --char 7

    # a server-private command (executeCommand); print the declared list
    # first, then actually send it
    dev/lsp-probe.py --server slang-server --root demo/rtl \\
                     --file demo/rtl/top/soc_top.sv --command slang.getInstances \\
                     --command-arg alu

    # dedicated completion flow: declare snippet/resolve support, wait for
    # indexing, tally the field distribution, then send
    # completionItem/resolve on the first item
    dev/lsp-probe.py --file foo.sv --completion --line 16 --char 24

    # for servers that need a long indexing time (rust-analyzer's cold
    # start takes 30+ seconds)
    dev/lsp-probe.py --server rust-analyzer --language-id rust \\
                     --file src/main.rs --completion --line 7 --char 14 --wait 45

Default server is verible-verilog-ls (this project's primary language);
use --server to switch to something else.
"""

import argparse
import json
import os
import select
import subprocess
import sys
import time


class LspProbe:
    def __init__(self, server_cmd, root):
        self.proc = subprocess.Popen(
            server_cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            # bufsize=0 isn't a performance choice, it's required for
            # correctness -- see header lesson 6. With buffering, `select`
            # can only see the fd, not bytes Python has already pulled into
            # its own buffer, so "the response already arrived" gets
            # misjudged as TIMEOUT.
            bufsize=0,
        )
        if not self.proc.stdin or not self.proc.stdout:
            sys.exit("failed to open pipes to the language server")
        self.pin = self.proc.stdin
        self.pout = self.proc.stdout
        self.root = root
        self.next_id = 1

    def send(self, obj):
        body = json.dumps(obj).encode()
        self.pin.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
        self.pin.flush()

    def _read_exactly(self, n):
        """Read exactly N bytes.

        With `bufsize=0`, `self.pout` is a raw FileIO, and its `read(n)`
        makes only **one** syscall, which can return fewer bytes than n
        (how much a pipe hands over in one go is up to the kernel).
        Calling `json.loads(self.pout.read(length))` directly would blow up
        randomly on large responses -- especially likely when completion
        returns 45 items or documentSymbol returns an entire tree.
        """
        chunks = []
        while n > 0:
            chunk = self.pout.read(n)
            if not chunk:
                return b""  # EOF
            chunks.append(chunk)
            n -= len(chunk)
        return b"".join(chunks)

    def read(self):
        length = 0
        while True:
            line = self.pout.readline()
            if not line:
                return None
            line = line.strip()
            if line.lower().startswith(b"content-length:"):
                length = int(line.split(b":")[1])
            elif line == b"":
                break
        body = self._read_exactly(length)
        if not body:
            return None
        return json.loads(body)

    def request(self, method, params, budget=15):
        rid = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        for _ in range(budget):
            msg = self.read()
            if msg is None:
                return None
            if msg.get("id") == rid:
                return msg
        return None

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})

    def pump(self, seconds, want_id=None, verbose=False):
        """Read messages until we get the reply for WANT_ID or time out, and
        return that reply (or None).

        `request` uses "read at most N messages" as its cap, which is
        enough for a quiet server, but misjudges a server that indexes on
        startup (see header lesson 3): the budget gets used up by
        `$/progress` messages and silently returns None, which looks
        exactly like "unsupported"; and `read` itself **blocks forever**
        when there's no message.

        This layer polls with select instead, and **replies to requests
        the server sends** -- LSP is bidirectional, and rust-analyzer sends
        `window/workDoneProgress/create` to request a progress token; it
        won't move on without a reply. Replying with `result: null` is
        enough.
        """
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if not select.select([self.pout], [], [], 0.25)[0]:
                continue
            msg = self.read()
            if msg is None:
                return None  # EOF: the server died, this is not a timeout
            if "id" in msg and "method" in msg:
                # server -> client request: must reply, or it may just stop there
                self.send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
                continue
            if want_id is not None and msg.get("id") == want_id:
                return msg
            if verbose and msg.get("method") == "$/progress":
                value = msg["params"].get("value") or {}
                if value.get("kind") == "end":
                    print(f"  indexing done: {msg['params'].get('token')}")
        return None

    def request_pumped(self, method, params, seconds):
        """Send a request and wait for it with `pump`, returning None on timeout."""
        rid = self.next_id
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return self.pump(seconds, want_id=rid)

    def await_diagnostics(self, uri, budget=15):
        for _ in range(budget):
            msg = self.read()
            if msg is None:
                return []
            if msg.get("method") == "textDocument/publishDiagnostics":
                if msg["params"].get("uri") == uri:
                    return msg["params"]["diagnostics"]
        return []

    def await_diagnostics_all(self, seconds=8.0):
        """Collect publishDiagnostics for **every** URI, not just the primary file.

        M63 needs to answer "if a second file is opened with didOpen on the
        same connection, will the server also send diagnostics for it" --
        `await_diagnostics` filters by uri, and would read and discard the
        second file's diagnostics, making "sent but dropped" look identical
        to "never sent at all" in the output. This is the sixth instance of
        header lesson 5, "missing a dimension to ask about": the missing
        dimension is on the **observation side**, not in the request shape.

        Uses a time budget plus select rather than "read at most N
        messages": we **don't know** how many will arrive, and `read`
        blocks forever when there's nothing (header lesson 3). The first
        version was written as `for _ in range(15): self.read()`; verible
        went quiet after sending three, and the probe hung until it was
        killed externally -- exactly the shape lesson 3 was meant to
        prevent, replayed inside a new function.
        """
        seen = {}
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if not select.select([self.pout], [], [], 0.25)[0]:
                continue
            msg = self.read()
            if msg is None:
                break  # EOF: the server died
            if "id" in msg and "method" in msg:
                self.send({"jsonrpc": "2.0", "id": msg["id"], "result": None})
                continue
            if msg.get("method") == "textDocument/publishDiagnostics":
                p = msg["params"]
                seen.setdefault(p.get("uri"), []).append(len(p["diagnostics"]))
        return seen

    def close(self):
        self.proc.kill()


def show_position(text, line, char):
    """Print the character at (line, char) with its surrounding context --
    a mirror for spotting off-by-one errors.

    LSP's character field is spec'd as a UTF-16 code unit; this uses
    Python's character index as an approximation, which is equivalent for
    ASCII Verilog source. Lines containing astral-plane characters will be
    off.
    """
    lines = text.split("\n")
    if line >= len(lines):
        print(f"  !! line {line} is out of range for the file ({len(lines)} lines)")
        return
    src = lines[line]
    if char > len(src):
        print(f"  !! char {char} is beyond that line's length ({len(src)})")
        return
    here = src[char] if char < len(src) else "<end of line>"
    print(f"  line {line}: {src!r}")
    print(f"  char {char} points at: {here!r}   context: {src[max(0, char - 3):char + 4]!r}")
    if here in " \t;,()[]{}":
        print("  ⚠ this looks like a delimiter, not an identifier -- the "
              "position may be off (see header lesson 2)")


# method -> the corresponding ServerCapabilities key. In a horizontal sweep,
# look at both: declared and actual behavior **often disagree**, and they
# disagree in both directions (lesson 1).
SWEEP_METHODS = [
    ("textDocument/formatting", "documentFormattingProvider"),
    ("textDocument/rangeFormatting", "documentRangeFormattingProvider"),
    ("textDocument/hover", "hoverProvider"),
    ("textDocument/definition", "definitionProvider"),
    ("textDocument/documentSymbol", "documentSymbolProvider"),
    ("textDocument/documentHighlight", "documentHighlightProvider"),
    ("textDocument/references", "referencesProvider"),
    ("textDocument/codeAction", "codeActionProvider"),
    ("textDocument/rename", "renameProvider"),
    ("textDocument/completion", "completionProvider"),
    ("textDocument/documentLink", "documentLinkProvider"),
    ("textDocument/inlayHint", "inlayHintProvider"),
    ("textDocument/prepareCallHierarchy", "callHierarchyProvider"),
]


def report_generic(resp, args):
    """Print one response in a human-readable way, distinguishing the three
    shapes that can look like "unsupported".

    Pulled into a function because workspace-level methods take a different
    params path (lesson 5), but **the rules for reading the response are
    exactly the same** -- duplicating this would leave one copy to go stale.
    """
    if resp is None:
        print(f"!! TIMEOUT: no response at all within {args.timeout}s")
        print("   This is a **different** failure mode from \"got back an empty "
              "result\": the server accepted the request and went silent,")
        print("   not even a JSON-RPC error, and the client's pending callback "
              "never gets called.")
        print("   Before concluding anything, check whether capabilities has "
              "that key (printed above); if both hold at once,")
        print("   that's the shape M57 fixed.")
        return
    if "error" in resp:
        err = resp["error"]
        print(f"!! JSON-RPC error {err.get('code')}: {err.get('message')}")
        return
    result = resp.get("result")
    print(json.dumps(result, indent=2, ensure_ascii=False))
    # Also catch "effectively empty": rename's no-result case looks like
    # {"changes": null}, not {} -- M48 got fooled by exactly this shape
    # once, see header lesson 2.
    blank = (None, [], {})
    if result in blank or (
        isinstance(result, dict) and result and all(v in blank for v in result.values())
    ):
        print("\n⚠ effectively empty response. Before concluding \"the "
              "server doesn't support this\", confirm the position above "
              "really points at an identifier.")


def sweep_params(method, line, char, tab_size, new_name, doc_lines=0):
    """What params a given method should be sent -- the differences in
    shape are bigger than you'd think, which is exactly why the old shared
    path (which hard-required a position) couldn't send formatting.

    DOC_LINES is the number of lines in the whole document; `inlayHint`
    uses it to compute the range for "the whole file"."""
    p = {}
    pos = {"line": line, "character": char}
    if method.endswith("/formatting"):
        p["options"] = {"tabSize": tab_size, "insertSpaces": True}
    elif method.endswith("/rangeFormatting"):
        p["options"] = {"tabSize": tab_size, "insertSpaces": True}
        p["range"] = {"start": {"line": line, "character": 0}, "end": {"line": line + 1, "character": 0}}
    elif method.endswith("/documentSymbol") or method.endswith("/documentLink"):
        pass  # only takes textDocument
    elif method.endswith("/inlayHint"):
        # No position, takes a range -- ask about "the whole file" to find
        # out whether it gives hints at all.
        p["range"] = {"start": {"line": 0, "character": 0}, "end": {"line": doc_lines, "character": 0}}
    elif method.endswith("/codeAction"):
        p["range"] = {"start": pos, "end": pos}
        p["context"] = {"diagnostics": []}
    else:
        p["position"] = pos
        if method.endswith("/rename"):
            p["newName"] = new_name
        if method.endswith("/references"):
            p["context"] = {"includeDeclaration": True}
    return p


def sweep(probe, uri, caps, args, doc_lines=0):
    """Send a set of methods once each and print a "declared vs actual" table.

    Why this is worth a dedicated mode instead of just telling the user to
    run it 10 times: this table is exactly what pared down M57's scope --
    reconnaissance pointed out that **seven** interactive commands shared
    the same "no timeout, silently stuck if no response comes" shape, and
    the natural impulse was to slap the fix on all seven; the sweep showed
    the real hole was only the two formatting methods, with the other five
    both declared and genuinely answered on both servers. **"The item says
    N methods" and "how many there actually are" have diverged too many
    times in this project** (three in M48, four in M53, two asymmetric
    consumers in M56); the friction of asking one by hand tempts you to
    only ask the ones the list names -- that's exactly where the misses
    come from.
    """
    print(f"\n=== horizontal sweep ({len(SWEEP_METHODS)} methods, {args.timeout}s cap each) ===")
    print(f"{'method':34s} {'declared':22s} actual result")
    for method, key in SWEEP_METHODS:
        declared = "<key missing>" if key not in caps else json.dumps(caps[key])
        if len(declared) > 20:
            declared = declared[:19] + "…"
        params = {"textDocument": {"uri": uri}}
        params.update(
            sweep_params(
                method, args.line, args.char, args.tab_size, args.new_name or "probe_new_name", doc_lines
            )
        )
        resp = probe.request_pumped(method, params, args.timeout)
        if resp is None:
            verdict = "!! TIMEOUT (accepted the request and went silent)"
        elif "error" in resp:
            verdict = f"error {resp['error'].get('code')}: {resp['error'].get('message', '')[:40]}"
        else:
            result = resp.get("result")
            if result is None:
                verdict = "result: null"
            elif isinstance(result, (list, dict)):
                verdict = f"result: {type(result).__name__}({len(result)})"
            else:
                verdict = f"result: {result!r}"
        print(f"{method:34s} {declared:22s} {verdict}")
    print("\nEvery cell where declared and actual disagree is worth stopping to look at: "
          "both directions have happened")
    print("(verible declares hoverProvider:false yet answers fine; slang-server declares "
          "everything")
    print(" yet never responds to formatting at all). TIMEOUT and error are **different** answers.")


def probe_completion(probe, uri, line, char, wait, trigger):
    """Dedicated completion flow: wait for indexing -> send the request
    (retrying if needed) -> tally the field distribution -> send
    `completionItem/resolve` on the first item, printing the diff between
    before and after resolve.

    Written separately instead of folded into the generic `--method` path,
    because completion has three things no other method has: (1) the
    client has to **declare** snippet/resolve support before the server
    will return those shapes; (2) the response is either a
    `CompletionList` or a bare array; (3) the real payoff -- documentation
    / detail / additionalTextEdits -- is often only available after
    resolve, and skipping resolve would misread it as "the server doesn't
    give documentation".
    """
    print(f"\n=== waiting for indexing (up to {wait}s) ===")
    probe.pump(wait, verbose=True)

    params = {
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": char},
        "context": ({"triggerKind": 2, "triggerCharacter": trigger} if trigger else {"triggerKind": 1}),
    }
    items = None
    for attempt in range(4):
        resp = probe.request_pumped("textDocument/completion", params, 20)
        if resp is None:
            print(f"  attempt {attempt}: no response (timeout)")
            continue
        if resp.get("error"):
            print(f"  server returned an error: {resp['error']}")
            return
        result = resp.get("result")
        items = result.get("items") if isinstance(result, dict) else result
        if items:
            break
        print(f"  attempt {attempt}: empty response, retrying in 5s")
        probe.pump(5)

    if not items:
        print("\n=== completion response: 0 items ===")
        print("⚠ Before concluding \"the server doesn't support this\", confirm the "
              "position above really points at an identifier,")
        print("  and confirm --wait was long enough (see header lesson 3: unfinished "
              "indexing looks exactly like unsupported).")
        return

    fmt = {}
    for it in items:
        key = it.get("insertTextFormat")
        fmt[key] = fmt.get(key, 0) + 1
    n = len(items)
    print(f"\n=== completion response: {n} items ===")
    print(f"insertTextFormat distribution (2=Snippet, 1/None=PlainText): {fmt}")
    for field in ("documentation", "detail", "additionalTextEdits", "textEdit", "filterText", "sortText"):
        print(f"  has {field:<20}: {sum(1 for it in items if it.get(field))}/{n}")
    print("\n--- first 3 items (before resolve) ---")
    print(json.dumps(items[:3], indent=2, ensure_ascii=False))

    target = next((it for it in items if not it.get("documentation")), items[0])
    resolved = probe.request_pumped("completionItem/resolve", target, 20)
    print("\n=== completionItem/resolve ===")
    if resolved is None:
        print("no response (timeout) -- the server may not support resolve.")
        return
    if resolved.get("error"):
        print(f"server returned an error: {resolved['error']}")
        return
    after = resolved.get("result") or {}
    gained = [k for k in after if k not in target or after[k] != target.get(k)]
    print(f"fields added/changed by resolve: {gained or '(none -- resolve is a no-op)'}")
    print(json.dumps(after, indent=2, ensure_ascii=False))


def parse_command_args(raw):
    """`--command-arg` receives strings, but executeCommand's arguments can
    be arbitrary JSON.

    If it parses as JSON, use the parsed value (so objects, arrays, numbers
    can actually be sent); otherwise treat it as a plain string -- private
    commands often just take a single URI or module name.
    """
    out = []
    for item in raw:
        try:
            out.append(json.loads(item))
        except json.JSONDecodeError:
            out.append(item)
    return out


def probe_call_hierarchy(probe, uri, line, char, timeout):
    """callHierarchy is **two steps**: first `prepareCallHierarchy` to get
    an item, then use that item to ask incoming/outgoing.

    Written separately instead of going through the generic `--method`
    path, because the second step's params are `{"item": ...}` --
    **it has neither textDocument nor position**, while the generic path
    builds params starting from exactly those two fields (header lesson 5).
    Sending `callHierarchy/incomingCalls` on its own never gets a real
    answer: the shape is simply wrong, and whatever the server replies
    can't be treated as evidence.
    """
    params = {"textDocument": {"uri": uri}, "position": {"line": line, "character": char}}
    resp = probe.request_pumped("textDocument/prepareCallHierarchy", params, timeout)
    print("\n=== step 1: textDocument/prepareCallHierarchy ===")
    if resp is None:
        print(f"!! TIMEOUT: no response at all within {timeout}s. Step 2 has nothing to ask with.")
        return
    if "error" in resp:
        err = resp["error"]
        print(f"!! JSON-RPC error {err.get('code')}: {err.get('message')}")
        return
    items = resp.get("result")
    print(json.dumps(items, indent=2, ensure_ascii=False))
    if not items:
        print("\n⚠ prepare returned empty -- step 2 has nothing to ask with (either "
              "this position isn't something callable,")
        print("  or the server merely declares callHierarchyProvider). Try a different "
              "position to tell which.")
        return
    for method in ("callHierarchy/incomingCalls", "callHierarchy/outgoingCalls"):
        sub = probe.request_pumped(method, {"item": items[0]}, timeout)
        print(f"\n=== step 2: {method}(item[0]) ===")
        if sub is None:
            print(f"!! TIMEOUT: no response at all within {timeout}s")
            continue
        if "error" in sub:
            err = sub["error"]
            print(f"!! JSON-RPC error {err.get('code')}: {err.get('message')}")
            continue
        print(json.dumps(sub.get("result"), indent=2, ensure_ascii=False))


def probe_execute_command(probe, caps, command, cmd_args, timeout):
    """`workspace/executeCommand` takes `{command, arguments}` -- completely
    different from `workspace/symbol`'s `{query}`, so it can't share the
    `workspace/` branch (header lesson 5).

    Prints the server's declared command list first: private command names
    are defined by the server itself, and a typo will just get you an
    error that looks like "unsupported".
    """
    declared = (caps.get("executeCommandProvider") or {}).get("commands") or []
    print(f"\n=== commands declared by the server ({len(declared)}) ===")
    for c in declared:
        print(f"  {c}")
    if command not in declared:
        print(f"\n⚠ {command!r} is not in the declared list -- sending it anyway "
              "(declarations are untrustworthy in both directions, see lesson 1),")
        print("  but on error, suspect a typo in the name first, not that it's unsupported.")
    params = {"command": command, "arguments": cmd_args}
    print(f"\n=== sending === {json.dumps(params, ensure_ascii=False)}")
    resp = probe.request_pumped("workspace/executeCommand", params, timeout)
    print("\n=== workspace/executeCommand response ===")
    if resp is None:
        print(f"!! TIMEOUT: no response at all within {timeout}s")
        return
    if "error" in resp:
        err = resp["error"]
        print(f"!! JSON-RPC error {err.get('code')}: {err.get('message')}")
        return
    print(json.dumps(resp.get("result"), indent=2, ensure_ascii=False))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--server", default="verible-verilog-ls", help="server executable (default verible-verilog-ls)")
    ap.add_argument("--file", required=True, help="the source file to open")
    ap.add_argument("--language-id", default="systemverilog")
    ap.add_argument(
        "--root",
        help="the rootUri directory sent to initialize (default: --file's own directory). "
        "The editor sends lsp--project-root (searches upward for a marker like "
        ".git/verible.filelist), and a server that indexes the whole workspace "
        "(slang-server) returns empty under the wrong root, "
        "which looks like unsupported -- use this flag to mimic the editor's behavior",
    )
    ap.add_argument("--method", help="the LSP method to send, e.g. textDocument/documentHighlight")
    ap.add_argument("--line", type=int, help="0-based line number")
    ap.add_argument("--char", type=int, help="0-based character position")
    ap.add_argument("--new-name", help="the new name for textDocument/rename")
    ap.add_argument(
        "--query",
        help="the query string for workspace/symbol (an empty string is legal, "
        "meaning \"everything\"). Workspace-level methods have no textDocument, "
        "see header lesson 5",
    )
    ap.add_argument("--at-diagnostic", type=int, help="use the Nth diagnostic's range as the request range (for codeAction)")
    ap.add_argument("--diagnostics", action="store_true", help="print publishDiagnostics (only for the primary file)")
    ap.add_argument(
        "--diagnostics-all",
        action="store_true",
        help="print publishDiagnostics stats for **every** URI. Use with `--also-open` to "
        "answer \"does the server take the second didOpen on the same connection "
        "seriously\" -- `--diagnostics` reads and discards other URIs, making \"sent "
        "but dropped\" look like \"never sent\"",
    )
    ap.add_argument(
        "--completion",
        action="store_true",
        help="dedicated completion flow: declare snippet/resolve support, wait for "
        "indexing, tally field distribution, then send completionItem/resolve",
    )
    ap.add_argument("--wait", type=int, default=5, help="seconds to wait for indexing after didOpen (rust-analyzer's cold start needs 30+, default 5)")
    ap.add_argument(
        "--timeout",
        type=float,
        default=10.0,
        help="cap in seconds for waiting on a --method response (default 10). "
        "Prints TIMEOUT on timeout instead of hanging forever -- "
        "\"the server accepts the request but never replies\" is a real failure mode "
        "(M57: that's exactly what slang-server does for "
        "textDocument/formatting), see header lesson 4",
    )
    ap.add_argument("--tab-size", type=int, default=2, help="FormattingOptions.tabSize for formatting (default 2)")
    ap.add_argument(
        "--sweep",
        action="store_true",
        help="horizontal sweep: send thirteen common methods once each, and print a "
        "\"declared vs actual\" table. Needs --line/--char (for position-based methods). "
        "This table is what pared down M57's scope",
    )
    ap.add_argument("--trigger-char", help="completion's triggerCharacter, e.g. . or :")
    ap.add_argument(
        "--call-hierarchy",
        action="store_true",
        help="dedicated callHierarchy flow: prepareCallHierarchy to get an item, then send "
        "incomingCalls / outgoingCalls with that item. Step 2's params are {item}, with "
        "no textDocument, so the generic --method path can't send the right shape",
    )
    ap.add_argument(
        "--command",
        help="the command name for workspace/executeCommand (e.g. slang.getInstances). "
        "It takes {command, arguments}, a different shape from workspace/symbol's {query}",
    )
    ap.add_argument(
        "--command-arg",
        action="append",
        default=[],
        metavar="JSON",
        help="a single argument for executeCommand, as a JSON literal (repeatable). "
        "Non-JSON strings are sent as plain text",
    )
    ap.add_argument(
        "--also-open",
        action="append",
        default=[],
        metavar="FILE",
        help="before sending the main request, send a textDocument/didOpen for this file "
        "first (repeatable) -- asks whether the server can see anything outside its "
        "\"known file view\" (M56: the dimension M55 probed with a throwaway script "
        "and then lost track of)",
    )
    args = ap.parse_args()

    if args.completion and (args.line is None or args.char is None):
        sys.exit("--completion requires --line/--char")
    if args.sweep and (args.line is None or args.char is None):
        sys.exit("--sweep requires --line/--char (needed for position-based methods like hover/definition)")
    if args.call_hierarchy and (args.line is None or args.char is None):
        sys.exit("--call-hierarchy requires --line/--char (prepareCallHierarchy is position-based)")

    path = os.path.abspath(args.file)
    if not os.path.exists(path):
        sys.exit(f"no such file: {path}")
    with open(path, encoding="utf-8") as f:
        text = f.read()
    uri = "file://" + path

    probe = LspProbe([args.server], os.path.dirname(path))
    try:
        # Declared client capabilities change the shape of what the server
        # returns, so this deliberately only adds the completion block when
        # --completion is given -- "what will reticle actually receive
        # right now" and "what's the most this server can possibly give" are
        # two different questions, don't mix them.
        text_document_caps = {
            "codeAction": {
                "codeActionLiteralSupport": {"codeActionKind": {"valueSet": ["quickfix", "refactor", "source"]}}
            },
            "rename": {"prepareSupport": True},
        }
        if args.completion:
            text_document_caps["completion"] = {
                "contextSupport": True,
                "completionItem": {
                    "snippetSupport": True,
                    "documentationFormat": ["markdown", "plaintext"],
                    "resolveSupport": {"properties": ["documentation", "detail", "additionalTextEdits"]},
                },
            }
        init = probe.request(
            "initialize",
            {
                "processId": os.getpid(),
                # `--root' matters more than it looks: a server that
                # indexes "the workspace" (slang-server does, by default
                # every .sv/.svh/.v/.vh under rootUri) answers NOTHING
                # cross-file when rootUri is just the file's own
                # directory -- and the editor does not send that. It
                # sends `lsp--project-root', the nearest ancestor
                # holding a marker (.git, verible.filelist, ...). Probing
                # with the default here therefore measures a root the
                # editor never uses, and reads as "the server can't do
                # it" when the real answer is "it was never told".
                "rootUri": "file://" + os.path.abspath(args.root or os.path.dirname(path)),
                "capabilities": {"textDocument": text_document_caps},
            },
        )
        if not init:
            sys.exit("no initialize response")
        caps = init["result"]["capabilities"]
        print("=== capabilities (declared values, untrustworthy in both directions, see header lesson 1) ===")
        print(json.dumps(caps, indent=2, sort_keys=True))

        probe.notify("initialized", {})
        probe.notify(
            "textDocument/didOpen",
            {
                "textDocument": {
                    "uri": uri,
                    "languageId": args.language_id,
                    "version": 1,
                    "text": text,
                }
            },
        )

        if args.also_open:
            print(f"\n=== extra didOpen ({len(args.also_open)} file(s), before the main request) ===")
            for extra in args.also_open:
                extra_path = os.path.abspath(extra)
                if not os.path.exists(extra_path):
                    sys.exit(f"--also-open: no such file: {extra_path}")
                with open(extra_path, encoding="utf-8") as f:
                    extra_text = f.read()
                probe.notify(
                    "textDocument/didOpen",
                    {
                        "textDocument": {
                            "uri": "file://" + extra_path,
                            "languageId": args.language_id,
                            "version": 1,
                            "text": extra_text,
                        }
                    },
                )
                print(f"  didOpen: {extra_path}")

        diags = []
        if args.diagnostics_all:
            seen = probe.await_diagnostics_all()
            print(f"\n=== publishDiagnostics: all URIs ({len(seen)}) ===")
            for u, counts in seen.items():
                print(f"  {u}  -> {len(counts)} notification(s), diagnostic counts {counts}")
            if not seen:
                print("  (none received within budget)")
        elif args.diagnostics or args.at_diagnostic is not None:
            diags = probe.await_diagnostics(uri)
            print(f"\n=== publishDiagnostics ({len(diags)} item(s)) ===")
            print(json.dumps(diags, indent=2, ensure_ascii=False))

        if args.sweep:
            print(f"\n=== sending position (line={args.line}, char={args.char}) ===")
            show_position(text, args.line, args.char)
            sweep(probe, uri, caps, args, doc_lines=len(text.split("\n")))
            return

        if args.call_hierarchy:
            print(f"\n=== sending position (line={args.line}, char={args.char}) ===")
            show_position(text, args.line, args.char)
            probe_call_hierarchy(probe, uri, args.line, args.char, args.timeout)
            return

        if args.command:
            probe_execute_command(probe, caps, args.command, parse_command_args(args.command_arg), args.timeout)
            return

        if args.completion:
            print(f"\n=== sending position (line={args.line}, char={args.char}) ===")
            show_position(text, args.line, args.char)
            print("(client has declared snippetSupport / resolveSupport -- this is "
                  "\"the most it can possibly give\",")
            print(" not what reticle currently receives while declaring empty capabilities)")
            probe_completion(probe, uri, args.line, args.char, args.wait, args.trigger_char)
            return

        if not args.method:
            return

        # Workspace-level methods **don't even have textDocument** -- see
        # header lesson 5. This branch must come before the shared
        # `params = {"textDocument": ...}`, or the params sent would carry
        # an extra field the server never asked for. The file still needs
        # a didOpen (already done above): verible needs an open file
        # present before it will load the project.
        if args.method == "workspace/executeCommand":
            sys.exit("use --command for workspace/executeCommand (it takes {command, arguments}, not {query})")
        if args.method.startswith("workspace/"):
            params = {"query": args.query or ""}
            print(f"\n=== workspace query string === {params['query']!r}")
            resp = probe.request_pumped(args.method, params, args.timeout)
            print(f"\n=== {args.method} response ===")
            report_generic(resp, args)
            return

        params = {"textDocument": {"uri": uri}}
        # The formatting family **has no** position: the whole-document
        # version only takes FormattingOptions, the range version takes
        # range + options. The old version hard-required "--line/--char" in
        # the shared path, so these two methods couldn't be sent at all --
        # M57 had to write a separate throwaway script just to find out how
        # slang-server handles formatting, exactly bypassing the reason
        # this tool exists.
        if args.method.endswith("/formatting") or args.method.endswith("/rangeFormatting"):
            params["options"] = {"tabSize": args.tab_size, "insertSpaces": True}
            if args.method.endswith("/rangeFormatting"):
                if args.line is None:
                    sys.exit("rangeFormatting requires --line (range is that whole line)")
                params["range"] = {
                    "start": {"line": args.line, "character": 0},
                    "end": {"line": args.line + 1, "character": 0},
                }
                print(f"\n=== sending range (whole line, line={args.line}) ===")
                show_position(text, args.line, args.char or 0)
        elif args.method.endswith("/documentLink"):
            pass  # only takes textDocument, no position -- adding one would break the shape from the spec
        elif args.method.endswith("/inlayHint"):
            # No position, takes a range. Defaults to the whole file: a
            # hint answers "what's in this span"; probing a single point
            # would misread "nothing here" as "the server refuses to give it".
            params["range"] = {
                "start": {"line": 0, "character": 0},
                "end": {"line": len(text.split("\n")), "character": 0},
            }
            print(f"\n=== sending range (whole file, {len(text.split(chr(10)))} lines) ===")
        elif args.at_diagnostic is not None:
            if args.at_diagnostic >= len(diags):
                sys.exit(f"only {len(diags)} diagnostic(s) available, can't get item {args.at_diagnostic}")
            d = diags[args.at_diagnostic]
            params["range"] = d["range"]
            params["context"] = {"diagnostics": [d]}
        elif args.line is not None and args.char is not None:
            print(f"\n=== sending position (line={args.line}, char={args.char}) ===")
            show_position(text, args.line, args.char)
            params["position"] = {"line": args.line, "character": args.char}
        else:
            sys.exit("need --line/--char or --at-diagnostic")

        if args.method.endswith("/rename"):
            if not args.new_name:
                sys.exit("rename requires --new-name")
            params["newName"] = args.new_name
        if args.method.endswith("/references"):
            params["context"] = {"includeDeclaration": True}

        # `request` is a blocking read: if the server accepts the request
        # but never replies, it would **hang forever** (confirmed in
        # practice in M57 -- it had to be killed externally).
        # `request_pumped` polls with select and has a cap, returning None
        # on timeout -- see header lesson 4.
        resp = probe.request_pumped(args.method, params, args.timeout)
        print(f"\n=== {args.method} response ===")
        report_generic(resp, args)
    finally:
        probe.close()


if __name__ == "__main__":
    main()
