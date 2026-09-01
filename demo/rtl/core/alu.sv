// alu.sv -- parameterised arithmetic/logic unit with one pipeline stage.
//
// Ready/valid on both sides so it composes with the rest of the design
// without a separate handshake wrapper.

module alu #(
    parameter int unsigned DataWidth = soc_pkg::DataWidth,
    // Set to 0 for a purely combinational unit; the handshake stays
    // valid either way.
    parameter bit          Pipelined = 1'b1
) (
    input logic clk_i,
    input logic rst_ni,

    input  soc_pkg::alu_op_e                 op_i,
    input  logic             [DataWidth-1:0] operand_a_i,
    input  logic             [DataWidth-1:0] operand_b_i,
    input  logic                             valid_i,
    output logic                             ready_o,

    output logic [DataWidth-1:0] result_o,
    output logic                 zero_o,
    output logic                 valid_o,
    input  logic                 ready_i
);

  localparam int unsigned ShiftW = $clog2(DataWidth);

  logic [DataWidth-1:0] result_d, result_q;
  logic valid_d, valid_q;
  logic [ShiftW-1:0] shamt;

  assign shamt = operand_b_i[ShiftW-1:0];

  always_comb begin
    unique case (op_i)
      soc_pkg::AluAdd: result_d = operand_a_i + operand_b_i;
      soc_pkg::AluSub: result_d = operand_a_i - operand_b_i;
      soc_pkg::AluAnd: result_d = operand_a_i & operand_b_i;
      soc_pkg::AluOr: result_d = operand_a_i | operand_b_i;
      soc_pkg::AluXor: result_d = operand_a_i ^ operand_b_i;
      soc_pkg::AluSll: result_d = operand_a_i << shamt;
      soc_pkg::AluSrl: result_d = operand_a_i >> shamt;
      soc_pkg::AluSra: result_d = $signed(operand_a_i) >>> shamt;
      soc_pkg::AluSlt:
      result_d = {{DataWidth - 1{1'b0}}, $signed(operand_a_i) < $signed(operand_b_i)};
      soc_pkg::AluSltu: result_d = {{DataWidth - 1{1'b0}}, operand_a_i < operand_b_i};
      default: result_d = '0;
    endcase
  end

  // The output register is only allowed to take a new value when the
  // downstream side is not holding a result we have yet to hand over.
  assign valid_d = valid_i | (valid_q & ~ready_i);
  assign ready_o = ~valid_q | ready_i;

  if (Pipelined) begin : gen_pipelined
    always_ff @(posedge clk_i or negedge rst_ni) begin
      if (!rst_ni) begin
        result_q <= '0;
        valid_q  <= 1'b0;
      end else begin
        valid_q <= valid_d;
        if (valid_i && ready_o) begin
          result_q <= result_d;
        end
      end
    end

    assign result_o = result_q;
    assign valid_o  = valid_q;
  end else begin : gen_combinational
    assign result_o = result_d;
    assign valid_o  = valid_i;
  end

  assign zero_o = (result_o == '0);

endmodule : alu
