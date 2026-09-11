// gray_ctr.v -- binary-up/Gray-code counter, plain Verilog-2001, NON-ANSI
// port list.
//
// The classic CDC pointer shape (a FIFO's write or read pointer crossing
// into the other clock domain single-bit-safely needs Gray, not binary,
// encoding) -- and a genuinely non-ANSI header: `module gray_ctr (clk,
// rst_n, en, bin_count, gray_count);` names the ports, then `input'/
// `output' declare their direction in the body, unlike `fifo_sync.v''s
// ANSI style in this same directory. Both styles are ordinary industry
// Verilog-2001; this file exists so the editor has real non-ANSI
// material to be tested against (see M124's Part A/E).
//
// M126: `bin_count' is a bare, untyped `output' so `/*AUTOREG*/' below
// can fill in its own `reg' declaration -- AUTOREG ignores procedural
// drivers, so it still declares `bin_count' even though the `always'
// block is its only real driver. `gray_count' keeps its explicit
// `output wire', which is why AUTOREG leaves it alone: a net-type
// keyword already on the port means there is nothing left to add.

`timescale 1ns / 1ps

module gray_ctr (
    clk,
    rst_n,
    en,
    bin_count,
    gray_count
);

  parameter WIDTH = 4;

  input clk;
  input rst_n;
  input en;
  output [WIDTH-1:0] bin_count;
  output wire [WIDTH-1:0] gray_count;

  /*AUTOREG*/
  // Beginning of automatic regs (for this module's undeclared outputs)
  reg [WIDTH-1:0] bin_count;
  // End of automatics

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      bin_count <= {WIDTH{1'b0}};
    end else if (en) begin
      bin_count <= bin_count + 1'b1;
    end
  end

  // Binary-to-Gray: XOR each bit with the one above it. A continuous
  // `assign' rather than `always @(*)' -- verible's default style guide
  // wants `always_comb' for combinational always-blocks, which is
  // SystemVerilog-only, so an `assign' is the correct Verilog-2001 idiom
  // here rather than a rule this directory would need to waive.
  assign gray_count = (bin_count >> 1) ^ bin_count;

endmodule
