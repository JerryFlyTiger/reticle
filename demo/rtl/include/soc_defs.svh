// soc_defs.svh -- project-wide preprocessor definitions.
//
// Kept deliberately small: macros hide structure from every tool that
// reads the source (including this editor's tree-sitter parser), so the
// design uses `soc_pkg' parameters and typedefs for everything that can
// be expressed in the language itself, and macros only for the things
// that genuinely cannot be.

`ifndef SOC_DEFS_SVH
`define SOC_DEFS_SVH

// Reset polarity is active-low across the whole design; these two names
// exist so a port list reads the same everywhere.
`define SOC_RST_N rst_ni
`define SOC_CLK clk_i

// Simulation-only assertion wrapper. Synthesis tools skip the whole
// block, so design files can carry their own checks without a separate
// bind file.
`ifdef SIMULATION
`define SOC_ASSERT(name, prop, clk, rst_n)          \
  name : assert property (@(posedge clk) disable iff (!rst_n) prop) \
    else $error("Assertion %s failed", `"name`");
`else
`define SOC_ASSERT(name, prop, clk, rst_n)
`endif

`endif  // SOC_DEFS_SVH
