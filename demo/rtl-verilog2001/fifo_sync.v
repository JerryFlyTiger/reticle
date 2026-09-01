// fifo_sync.v -- synchronous FIFO, plain Verilog-2001.
//
// Deliberately NOT SystemVerilog: `reg'/`wire' instead of `logic', no
// `always_ff', no packed structs. Plenty of production RTL still looks
// exactly like this, and `.v' files take the same editing path in
// reticle as `.sv' does -- IEEE 1364 was folded into IEEE 1800 in
// 2009, so one parser covers both.

`timescale 1ns / 1ps

module fifo_sync #(
    parameter DATA_WIDTH = 32,
    parameter DEPTH      = 16,
    parameter ADDR_WIDTH = 4    // must satisfy 2**ADDR_WIDTH == DEPTH
) (
    input wire clk,
    input wire rst_n,

    input  wire                  wr_en,
    input  wire [DATA_WIDTH-1:0] wr_data,
    output wire                  full,

    input  wire                  rd_en,
    output wire [DATA_WIDTH-1:0] rd_data,
    output wire                  empty,

    output wire [ADDR_WIDTH:0] level
);

  reg [DATA_WIDTH-1:0] mem[0:DEPTH-1];

  reg [ADDR_WIDTH:0] wr_ptr;
  reg [ADDR_WIDTH:0] rd_ptr;

  wire do_write = wr_en && !full;
  wire do_read = rd_en && !empty;

  // One extra pointer bit distinguishes full from empty when the two
  // pointers land on the same entry -- the classic Verilog FIFO trick.
  assign empty = (wr_ptr == rd_ptr);
  assign full  = (wr_ptr[ADDR_WIDTH] != rd_ptr[ADDR_WIDTH]) &&
                 (wr_ptr[ADDR_WIDTH-1:0] == rd_ptr[ADDR_WIDTH-1:0]);
  assign level = wr_ptr - rd_ptr;

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      wr_ptr <= {(ADDR_WIDTH + 1) {1'b0}};
    end else if (do_write) begin
      wr_ptr <= wr_ptr + 1'b1;
    end
  end

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      rd_ptr <= {(ADDR_WIDTH + 1) {1'b0}};
    end else if (do_read) begin
      rd_ptr <= rd_ptr + 1'b1;
    end
  end

  always @(posedge clk) begin
    if (do_write) begin
      mem[wr_ptr[ADDR_WIDTH-1:0]] <= wr_data;
    end
  end

  assign rd_data = mem[rd_ptr[ADDR_WIDTH-1:0]];

endmodule
