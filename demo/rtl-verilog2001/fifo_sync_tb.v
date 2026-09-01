// fifo_sync_tb.v -- testbench for fifo_sync, plain Verilog-2001.
//
// Runs under Icarus Verilog:
//     iverilog -o /tmp/fifo_tb fifo_sync.v fifo_sync_tb.v && /tmp/fifo_tb
//
// Included because a demo that only shows synthesisable RTL leaves out
// half of what an RTL engineer actually edits all day.

`timescale 1ns / 1ps

module fifo_sync_tb;

  localparam DATA_WIDTH = 32;
  localparam DEPTH      = 8;
  localparam ADDR_WIDTH = 3;

  reg                      clk;
  reg                      rst_n;
  reg                      wr_en;
  reg     [DATA_WIDTH-1:0] wr_data;
  reg                      rd_en;
  wire    [DATA_WIDTH-1:0] rd_data;
  wire                     full;
  wire                     empty;
  wire    [  ADDR_WIDTH:0] level;

  integer                  i;
  integer                  errors;

  fifo_sync #(
      .DATA_WIDTH(DATA_WIDTH),
      .DEPTH     (DEPTH),
      .ADDR_WIDTH(ADDR_WIDTH)
  ) dut (
      .clk    (clk),
      .rst_n  (rst_n),
      .wr_en  (wr_en),
      .wr_data(wr_data),
      .full   (full),
      .rd_en  (rd_en),
      .rd_data(rd_data),
      .empty  (empty),
      .level  (level)
  );

  always #5 clk = ~clk;

  task push;
    input [DATA_WIDTH-1:0] value;
    begin
      @(negedge clk);
      wr_en   = 1'b1;
      wr_data = value;
      @(negedge clk);
      wr_en = 1'b0;
    end
  endtask

  task pop;
    output [DATA_WIDTH-1:0] value;
    begin
      @(negedge clk);
      rd_en = 1'b1;
      value = rd_data;
      @(negedge clk);
      rd_en = 1'b0;
    end
  endtask

  reg [DATA_WIDTH-1:0] got;

  initial begin
    clk     = 1'b0;
    rst_n   = 1'b0;
    wr_en   = 1'b0;
    rd_en   = 1'b0;
    wr_data = {DATA_WIDTH{1'b0}};
    errors  = 0;

    repeat (2) @(negedge clk);
    rst_n = 1'b1;

    if (!empty) begin
      $display("FAIL: FIFO not empty after reset");
      errors = errors + 1;
    end

    // Fill it right up to full, then check the flag.
    for (i = 0; i < DEPTH; i = i + 1) begin
      push(i[DATA_WIDTH-1:0]);
    end

    if (!full) begin
      $display("FAIL: FIFO not full after %0d pushes", DEPTH);
      errors = errors + 1;
    end

    // Drain it, checking FIFO order on the way out.
    for (i = 0; i < DEPTH; i = i + 1) begin
      pop(got);
      if (got !== i[DATA_WIDTH-1:0]) begin
        $display("FAIL: entry %0d read back as %0d", i, got);
        errors = errors + 1;
      end
    end

    if (!empty) begin
      $display("FAIL: FIFO not empty after draining");
      errors = errors + 1;
    end

    if (errors == 0) begin
      $display("PASS: fifo_sync %0d x %0d", DEPTH, DATA_WIDTH);
    end else begin
      $display("%0d failure(s)", errors);
    end

    $finish;
  end

endmodule
