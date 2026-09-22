// exec_unit.sv -- register file feeding the ALU, as one execution slice.
//
// Showcase for two AUTO features together: AUTOINPUT/AUTOOUTPUT filling in
// this module's own port list from its sub-instances' unconnected ports
// (M150 -- notice the two "Beginning of automatic" banners sit INSIDE the
// `)(' parentheses below, not after them), and AUTOWIRE declaring the two
// internal `operand_a'/`operand_b' wires the AUTO_TEMPLATEs below route
// between `u_regfile' and `u_alu' (M152 -- each carries a bit/part-select
// through the template, `operand_\1[]', not a bare connection).
//
// `op_i' is hand-written rather than AUTO-generated: reticle's own
// AUTOINPUT *does* propagate a package-scoped enum port like this one
// (measured -- deleting this line and re-running AUTOINPUT regenerates
// `input soc_pkg::alu_op_e op_i,' verbatim), but real GNU Emacs
// verilog-mode never does -- its AUTOINPUT only ever propagates plain
// vector ports. Keeping it hand-written here means the file expands the
// same way under either implementation, and shows the common real-world
// pattern of hand-declaring one control port while automating the
// routine data/handshake ones.

module exec_unit #(
    parameter int unsigned DataWidth = soc_pkg::DataWidth,
    parameter int unsigned NumRegs   = soc_pkg::RegCount
) (
    input soc_pkg::alu_op_e op_i,

    /*AUTOINPUT*/
    // Beginning of automatic inputs (from unused autoinst inputs)
    input  logic                         clk_i,      // To u_regfile of regfile.sv, ...
    input  logic [$clog2((NumRegs))-1:0] raddr_a_i,  // To u_regfile of regfile.sv
    input  logic [$clog2((NumRegs))-1:0] raddr_b_i,  // To u_regfile of regfile.sv
    input  logic                         ready_i,    // To u_alu of alu.sv
    input  logic                         rst_ni,     // To u_regfile of regfile.sv, ...
    input  logic                         valid_i,    // To u_alu of alu.sv
    input  logic [$clog2((NumRegs))-1:0] waddr_i,    // To u_regfile of regfile.sv
    input  logic [      (DataWidth)-1:0] wdata_i,    // To u_regfile of regfile.sv
    input  logic                         we_i,       // To u_regfile of regfile.sv
    // End of automatics
    /*AUTOOUTPUT*/
    // Beginning of automatic outputs (from unused autoinst outputs)
    output logic                         ready_o,    // From u_alu of alu.sv
    output logic [      (DataWidth)-1:0] result_o,   // From u_alu of alu.sv
    output logic                         valid_o,    // From u_alu of alu.sv
    output logic                         zero_o      // From u_alu of alu.sv
    // End of automatics
);

  /*AUTOWIRE*/
  // Beginning of automatic wires (for undeclared instantiated-module outputs)
  wire [DataWidth-1:0] operand_a;
  wire [DataWidth-1:0] operand_b;
  // End of automatics

  /* regfile AUTO_TEMPLATE (
      .rdata_\(.\)_o (operand_\1[]),
  ); */
  regfile #(
      .DataWidth(DataWidth),
      .NumRegs  (NumRegs)
  ) u_regfile (  /*AUTOINST*/
      // Outputs
      .rdata_a_o(operand_a[DataWidth-1:0]),          // Templated
      .rdata_b_o(operand_b[DataWidth-1:0]),          // Templated
      // Inputs
      .clk_i    (clk_i),
      .rst_ni   (rst_ni),
      .raddr_a_i(raddr_a_i[$clog2((NumRegs))-1:0]),
      .raddr_b_i(raddr_b_i[$clog2((NumRegs))-1:0]),
      .we_i     (we_i),
      .waddr_i  (waddr_i[$clog2((NumRegs))-1:0]),
      .wdata_i  (wdata_i[(DataWidth)-1:0])
  );

  /* alu AUTO_TEMPLATE (
      .operand_\(.\)_i (operand_\1[]),
  ); */
  alu #(
      .DataWidth(DataWidth)
  ) u_alu (  /*AUTOINST*/
      // Outputs
      .ready_o    (ready_o),
      .result_o   (result_o[(DataWidth)-1:0]),
      .zero_o     (zero_o),
      .valid_o    (valid_o),
      // Inputs
      .clk_i      (clk_i),
      .rst_ni     (rst_ni),
      .op_i       (op_i),
      .operand_a_i(operand_a[DataWidth-1:0]),   // Templated
      .operand_b_i(operand_b[DataWidth-1:0]),   // Templated
      .valid_i    (valid_i),
      .ready_i    (ready_i)
  );

endmodule
