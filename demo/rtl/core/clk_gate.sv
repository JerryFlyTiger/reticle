// clk_gate.sv -- integrated clock gating cell (latch-based AND gate).
//
// A plain `assign gated_clk_o = clk_i & enable_i` would glitch: if
// `enable_i` changes while `clk_i` is high, the AND gate's output
// changes mid-cycle and every flop it drives sees a spurious extra
// edge. The textbook fix latches the enable while the clock is low, so
// it can only change while the gated clock is already low, and ANDs
// the latched value with the clock afterwards. `test_en_i` bypasses the
// latch entirely so scan shift clocks are never gated.

module clk_gate (
    input logic clk_i,
    input logic rst_ni,
    input logic enable_i,
    input logic test_en_i,

    output logic gated_clk_o
);

  logic enable_latched_q;

  // Transparent while clk_i is low, holds while clk_i is high -- the
  // one place in this design where a level-sensitive latch is the
  // correct primitive rather than a modelling shortcut.
  always_latch begin
    if (!rst_ni) begin
      enable_latched_q = 1'b0;
    end else if (!clk_i) begin
      enable_latched_q = enable_i | test_en_i;
    end
  end

  assign gated_clk_o = clk_i & (enable_latched_q | test_en_i);

endmodule : clk_gate
