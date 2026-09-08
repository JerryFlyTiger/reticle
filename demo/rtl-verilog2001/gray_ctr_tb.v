// gray_ctr_tb.v -- testbench for gray_ctr, plain Verilog-2001.
//
// Runs under Icarus Verilog:
//     iverilog -o /tmp/gray_ctr_tb gray_ctr.v gray_ctr_tb.v && /tmp/gray_ctr_tb
//
// Checks two things every cycle: the binary count increments exactly
// as expected, and the Gray-coded output always differs from the
// PREVIOUS Gray value in exactly one bit -- the whole point of a Gray
// counter for clock-domain crossing.
//
// The bit-count check is inlined below with a plain `for' loop rather
// than a `function' -- verible's default style guide wants an explicit
// `automatic'/`static' lifetime on every function, and this directory's
// whole point is showing what PLAIN Verilog-2001 looks like, so
// sidestepping the construct is preferable to waiving a fifth rule.

`timescale 1ns / 1ps

module gray_ctr_tb;

  localparam WIDTH = 4;

  reg clk;
  reg rst_n;
  reg en;
  wire [WIDTH-1:0] bin_count;
  wire [WIDTH-1:0] gray_count;

  reg [WIDTH-1:0] prev_gray;
  reg [WIDTH-1:0] expected_bin;
  reg [WIDTH-1:0] gray_diff;
  integer i;
  integer j;
  integer errors;
  integer changed_bits;

  gray_ctr #(
      .WIDTH(WIDTH)
  ) dut (
      .clk(clk),
      .rst_n(rst_n),
      .en(en),
      .bin_count(bin_count),
      .gray_count(gray_count)
  );

  always #5 clk = ~clk;

  initial begin
    clk = 1'b0;
    rst_n = 1'b0;
    en = 1'b0;
    errors = 0;

    repeat (2) @(negedge clk);
    rst_n = 1'b1;
    @(negedge clk);

    if (bin_count !== {WIDTH{1'b0}}) begin
      $display("FAIL: bin_count not zero after reset");
      errors = errors + 1;
    end
    if (gray_count !== {WIDTH{1'b0}}) begin
      $display("FAIL: gray_count not zero after reset");
      errors = errors + 1;
    end

    prev_gray = gray_count;
    en = 1'b1;
    expected_bin = {WIDTH{1'b0}};

    // Walk the counter through two full wraps, checking BOTH that the
    // binary count is exactly what's expected and that each Gray step
    // flips exactly one bit versus the previous cycle.
    for (i = 0; i < (2 * (1 << WIDTH)); i = i + 1) begin
      @(negedge clk);
      expected_bin = expected_bin + 1'b1;

      if (bin_count !== expected_bin) begin
        $display("FAIL: step %0d bin_count = %0d, expected %0d", i, bin_count, expected_bin);
        errors = errors + 1;
      end

      gray_diff = gray_count ^ prev_gray;
      changed_bits = 0;
      for (j = 0; j < WIDTH; j = j + 1) begin
        changed_bits = changed_bits + gray_diff[j];
      end
      if (changed_bits != 1) begin
        $display("FAIL: step %0d Gray step changed %0d bits (want 1): prev=%b now=%b", i,
                 changed_bits, prev_gray, gray_count);
        errors = errors + 1;
      end
      prev_gray = gray_count;
    end

    if (errors == 0) begin
      $display("PASS: gray_ctr WIDTH=%0d", WIDTH);
    end else begin
      $display("%0d failure(s)", errors);
    end

    $finish;
  end

endmodule
