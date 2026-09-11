// status_regs_stub.sv -- bring-up stub for a status/debug register block:
// an ANSI SystemVerilog module whose outputs are declared but not yet
// driven, ordinary early-stage RTL practice.
//
// /*AUTOTIEOFF*/ below fills each output with a constant zero until the
// real logic lands. `error_o` and `mode_o` show the plain numeric-range
// forms; `status_o` shows the `[N-1:0]` special case, `{DataWidth{1'b0}}`.
// On an ANSI header (all of `demo/rtl/`) the tie-off uses `assign` (M126)
// rather than a body `wire`, which would duplicate the port declaration
// and not compile.

module status_regs_stub #(
    parameter int unsigned DataWidth = 32
) (
    input  logic                 clk_i,
    input  logic                 rst_ni,
    output logic [DataWidth-1:0] status_o,
    output logic                 error_o,
    output logic [          1:0] mode_o
);

  /*AUTOTIEOFF*/
  // Beginning of automatic tieoffs (for this module's unterminated outputs)
  assign error_o  = 1'h0;
  assign mode_o   = 2'h0;
  assign status_o = {DataWidth{1'b0}};
  // End of automatics

endmodule
