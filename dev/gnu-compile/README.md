# GNU Emacs reference: compile errors printed from a subdirectory (M141)

`run.sh` builds a throwaway project whose top `Makefile` runs `$(MAKE) -C sim`,
captures real logs from macOS make 3.81 (plain and `-w`) and gmake 4.x when
installed, adds one synthetic log with nested `Entering`/`Leaving directory`
lines, and runs `probe.el` under `emacs -Q --batch` over all of them.

    dev/gnu-compile/run.sh > dev/gnu-compile/run.out

`run.out` is the output from 2026-09-14 (GNU Emacs 30.2, GNU Make 3.81,
GNU Make 4.4.1). The project path is printed as `<P>`.

What it shows, and what reticle's `compile.el` does with it:

- make 3.81 without `-w` prints no directory line. GNU keeps the error with no
  directory (`dir=nil`, the file does not exist). reticle drops it, because
  the existence check is its false-positive filter, and counts it in the
  finish message.
- make 3.81 `-w` (`` `...' `` quoting) and gmake 4.x (`'...'` quoting) announce
  the directory. GNU resolves the error into `sim/`. reticle does the same.
- In the nested log GNU pops on `Leaving`, and resolves the relative
  `Entering directory 'deep'` against the buffer's directory (`<P>/deep`), not
  against the directory it was in (`<P>/sim`). reticle deliberately resolves it
  against the current directory instead (`<P>/sim/deep`). Real make prints
  absolute paths, so this only matters for other tools that imitate the line.
