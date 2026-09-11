// fifo_gray_top.v -- non-ANSI top wrapping fifo_sync (ANSI submodule) and
// gray_ctr (non-ANSI submodule), M125's demo material for
// /*AUTOOUTPUT*/, /*AUTOINPUT*/ and /*AUTOINOUT*/: two instances share
// `clk' and `rst_n', so the AUTOINPUT block below has to collapse both
// contributions into one declaration. The file is checked in fully
// expanded -- markers and generated blocks both -- and pinned
// byte-for-byte against the editor's own output by
// `demo_verilog2001_auto_top_matches_editor_output' in
// crates/core/tests/demo_smoke_tests.rs.
//
// The design itself is minimal on purpose (the subject here is the AUTO
// expansion, not the datapath): the Gray counter tracks the FIFO's own
// write-enable, the ordinary CDC-pointer use `gray_ctr.v''s own header in
// this directory describes -- a write-side counter meant to cross into
// another clock domain single-bit-safely. Nothing else in this module
// consumes `bin_count' or `gray_count', so both reach the module boundary
// undriven-elsewhere and AUTOOUTPUT declares them as real top-level ports
// -- not AUTOWIRE internals, which is what a naive guess might expect.
// AUTOWIRE itself expands to nothing here (no candidate is left over once
// AUTOOUTPUT/AUTOINPUT have claimed everything), which is why `/*AUTOWIRE*/'
// below stays a bare, unexpanded marker.

`timescale 1ns / 1ps

module fifo_gray_top (  /*AUTOARG*/
    // Outputs
    bin_count,
    empty,
    full,
    gray_count,
    level,
    rd_data,
    // Inputs
    clk,
    rd_en,
    rst_n,
    wr_data,
    wr_en
);

  parameter DATA_WIDTH = 32;
  parameter DEPTH = 16;
  parameter ADDR_WIDTH = 4;  // must satisfy 2**ADDR_WIDTH == DEPTH
  parameter GRAY_WIDTH = 4;

  /*AUTOOUTPUT*/
  // Beginning of automatic outputs (from unused autoinst outputs)
  output [(GRAY_WIDTH)-1:0] bin_count;  // From u_gray_ctr of gray_ctr.v
  output empty;  // From u_fifo_sync of fifo_sync.v
  output full;  // From u_fifo_sync of fifo_sync.v
  output [(GRAY_WIDTH)-1:0] gray_count;  // From u_gray_ctr of gray_ctr.v
  output [(ADDR_WIDTH):0] level;  // From u_fifo_sync of fifo_sync.v
  output [(DATA_WIDTH)-1:0] rd_data;  // From u_fifo_sync of fifo_sync.v
  // End of automatics
  /*AUTOINPUT*/
  // Beginning of automatic inputs (from unused autoinst inputs)
  input clk;  // To u_fifo_sync of fifo_sync.v, ...
  input rd_en;  // To u_fifo_sync of fifo_sync.v
  input rst_n;  // To u_fifo_sync of fifo_sync.v, ...
  input [(DATA_WIDTH)-1:0] wr_data;  // To u_fifo_sync of fifo_sync.v
  input wr_en;  // To u_fifo_sync of fifo_sync.v, ...
  // End of automatics

  fifo_sync #(
      .DATA_WIDTH(DATA_WIDTH),
      .DEPTH(DEPTH),
      .ADDR_WIDTH(ADDR_WIDTH)
  ) u_fifo_sync (
      .clk    (clk),
      .rst_n  (rst_n),
      /*AUTOINST*/
      // Outputs
      .full   (full),
      .rd_data(rd_data[(DATA_WIDTH)-1:0]),
      .empty  (empty),
      .level  (level[(ADDR_WIDTH):0]),
      // Inputs
      .wr_en  (wr_en),
      .wr_data(wr_data[(DATA_WIDTH)-1:0]),
      .rd_en  (rd_en)
  );

  gray_ctr #(
      .WIDTH(GRAY_WIDTH)
  ) u_gray_ctr (
      .clk       (clk),
      .rst_n     (rst_n),
      .en        (wr_en),
      /*AUTOINST*/
      // Outputs
      .bin_count (bin_count[(GRAY_WIDTH)-1:0]),
      .gray_count(gray_count[(GRAY_WIDTH)-1:0])
  );

  /*AUTOWIRE*/

endmodule
