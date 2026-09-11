// fifo_gray_top_tb.v -- testbench for fifo_gray_top, plain Verilog-2001.
//
// Runs under Icarus Verilog:
//     iverilog -o /tmp/fifo_gray_top_tb fifo_sync.v gray_ctr.v \
//       fifo_gray_top.v fifo_gray_top_tb.v && /tmp/fifo_gray_top_tb
//
// Checks the FIFO behaves exactly like `fifo_sync_tb.v''s own DUT (push
// to full, drain in order, empty again) AND that `bin_count'/
// `gray_count' -- the two AUTOOUTPUT-declared ports this file exists to
// exercise -- actually track every write: `bin_count' increments by
// exactly one per push, and each Gray step differs from the previous one
// in exactly one bit, the same property `gray_ctr_tb.v' checks directly
// on `gray_ctr' alone.

`timescale 1ns / 1ps

module fifo_gray_top_tb;

  localparam DATA_WIDTH = 32;
  localparam DEPTH = 8;
  localparam ADDR_WIDTH = 3;
  localparam GRAY_WIDTH = 4;

  reg clk;
  reg rst_n;
  reg wr_en;
  reg [DATA_WIDTH-1:0] wr_data;
  reg rd_en;
  wire [DATA_WIDTH-1:0] rd_data;
  wire full;
  wire empty;
  wire [ADDR_WIDTH:0] level;
  wire [GRAY_WIDTH-1:0] bin_count;
  wire [GRAY_WIDTH-1:0] gray_count;

  integer i;
  integer j;
  integer errors;
  reg [GRAY_WIDTH-1:0] expected_bin;
  reg [GRAY_WIDTH-1:0] prev_gray;
  reg [GRAY_WIDTH-1:0] gray_diff;
  integer changed_bits;

  fifo_gray_top #(
      .DATA_WIDTH(DATA_WIDTH),
      .DEPTH(DEPTH),
      .ADDR_WIDTH(ADDR_WIDTH),
      .GRAY_WIDTH(GRAY_WIDTH)
  ) dut (
      .clk(clk),
      .rst_n(rst_n),
      .wr_en(wr_en),
      .wr_data(wr_data),
      .full(full),
      .rd_en(rd_en),
      .rd_data(rd_data),
      .empty(empty),
      .level(level),
      .bin_count(bin_count),
      .gray_count(gray_count)
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
    clk = 1'b0;
    rst_n = 1'b0;
    wr_en = 1'b0;
    rd_en = 1'b0;
    wr_data = {DATA_WIDTH{1'b0}};
    errors = 0;

    repeat (2) @(negedge clk);
    rst_n = 1'b1;
    @(negedge clk);

    if (!empty) begin
      $display("FAIL: FIFO not empty after reset");
      errors = errors + 1;
    end
    if (bin_count !== {GRAY_WIDTH{1'b0}}) begin
      $display("FAIL: bin_count not zero after reset");
      errors = errors + 1;
    end
    if (gray_count !== {GRAY_WIDTH{1'b0}}) begin
      $display("FAIL: gray_count not zero after reset");
      errors = errors + 1;
    end

    expected_bin = {GRAY_WIDTH{1'b0}};
    prev_gray = gray_count;

    // Fill the FIFO right up to full, checking on every push that
    // bin_count/gray_count -- driven off wr_en, not rd_en -- tracked
    // that one write exactly once.
    for (i = 0; i < DEPTH; i = i + 1) begin
      push(i[DATA_WIDTH-1:0]);
      expected_bin = expected_bin + 1'b1;

      if (bin_count !== expected_bin) begin
        $display("FAIL: push %0d bin_count = %0d, expected %0d", i, bin_count, expected_bin);
        errors = errors + 1;
      end

      gray_diff = gray_count ^ prev_gray;
      changed_bits = 0;
      for (j = 0; j < GRAY_WIDTH; j = j + 1) begin
        changed_bits = changed_bits + gray_diff[j];
      end
      if (changed_bits != 1) begin
        $display("FAIL: push %0d Gray step changed %0d bits (want 1): prev=%b now=%b", i,
                 changed_bits, prev_gray, gray_count);
        errors = errors + 1;
      end
      prev_gray = gray_count;
    end

    if (!full) begin
      $display("FAIL: FIFO not full after %0d pushes", DEPTH);
      errors = errors + 1;
    end

    // Drain it, checking FIFO order on the way out. Reads must NOT move
    // bin_count/gray_count at all -- they are wired to wr_en, not rd_en.
    for (i = 0; i < DEPTH; i = i + 1) begin
      pop(got);
      if (got !== i[DATA_WIDTH-1:0]) begin
        $display("FAIL: entry %0d read back as %0d", i, got);
        errors = errors + 1;
      end
      if (bin_count !== expected_bin) begin
        $display("FAIL: pop %0d moved bin_count to %0d, expected unchanged %0d", i, bin_count,
                 expected_bin);
        errors = errors + 1;
      end
    end

    if (!empty) begin
      $display("FAIL: FIFO not empty after draining");
      errors = errors + 1;
    end

    if (errors == 0) begin
      $display("PASS: fifo_gray_top %0d x %0d, GRAY_WIDTH=%0d", DEPTH, DATA_WIDTH, GRAY_WIDTH);
    end else begin
      $display("%0d failure(s)", errors);
    end

    $finish;
  end

endmodule
