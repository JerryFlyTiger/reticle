// soc_top.sv -- the integration file, and the one worth opening first.
//
// Every submodule instantiated here lives in a DIFFERENT subdirectory
// (`core/', `mem/', `bus/'), which is the ordinary shape of an RTL tree
// and the reason reticle learned to search library files
// recursively (see PLAN.md, M56). With this file open:
//
//   * put the cursor on `alu' below and press `M-.'  -> jumps straight
//     to core/alu.sv, no language server needed.
//   * type `.' inside u_regfile's port list and press `C-M-i'  -> the
//     completion list is regfile's own port names, read out of
//     core/regfile.sv.
//   * `M-x verilog-auto' expands the /*AUTOINST*/ in u_arbiter into a
//     full port list, the way GNU verilog-mode's AUTO macros do.
//
// u_alu below is shown ALREADY expanded so the file reads as finished
// RTL; u_arbiter is left unexpanded so there is something to try.

`include "soc_defs.svh"

module soc_top
  import soc_pkg::*;
#(
    parameter int unsigned DataWidth = soc_pkg::DataWidth,
    parameter int unsigned SramAddrW = 12
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic          [NumMasters-1:0] host_req_valid_i,
    output logic          [NumMasters-1:0] host_req_ready_o,
    input  soc_pkg::req_t                  host_req_i      [NumMasters],

    input  alu_op_e                 alu_op_i,
    input  logic                    alu_valid_i,
    output logic                    alu_ready_o,
    output logic    [DataWidth-1:0] alu_result_o,
    output logic                    alu_valid_o,

    output logic [DataWidth-1:0] sram_rdata_o,
    output logic                 sram_rvalid_o
);

  logic [DataWidth-1:0] rs1_data, rs2_data;
  logic                                   alu_zero;

  soc_pkg::req_t                          gnt_req;
  logic                                   gnt_valid;
  logic          [$clog2(NumMasters)-1:0] gnt_idx;

  // --- register file -------------------------------------------------
  // Try `C-M-i' after a `.' anywhere in this port list.
  regfile #(
      .DataWidth(DataWidth),
      .NumRegs  (RegCount)
  ) u_regfile (
      .clk_i    (clk_i),
      .rst_ni   (rst_ni),
      .raddr_a_i(5'd1),
      .rdata_a_o(rs1_data),
      .raddr_b_i(5'd2),
      .rdata_b_o(rs2_data),
      .we_i     (alu_valid_o),
      .waddr_i  (5'd3),
      .wdata_i  (alu_result_o)
  );

  // --- ALU -----------------------------------------------------------
  // Put the cursor on the module name `alu' and press `M-.'.
  alu #(
      .DataWidth(DataWidth),
      .Pipelined(1'b1)
  ) u_alu (
      .clk_i      (clk_i),
      .rst_ni     (rst_ni),
      .op_i       (alu_op_i),
      .operand_a_i(rs1_data),
      .operand_b_i(rs2_data),
      .valid_i    (alu_valid_i),
      .ready_o    (alu_ready_o),
      .result_o   (alu_result_o),
      .zero_o     (alu_zero),
      .valid_o    (alu_valid_o),
      .ready_i    (1'b1)
  );

  // --- bus arbiter ---------------------------------------------------
  // Left unexpanded on purpose: run `M-x verilog-auto' here.
  axi4_lite_arbiter #(
      .NumMasters(NumMasters)
  ) u_arbiter (
  /*AUTOINST*/
  );

  // --- memory --------------------------------------------------------
  sram_wrapper #(
      .AddrWidth  (SramAddrW),
      .DataWidth  (DataWidth),
      .ReadLatency(1)
  ) u_sram (
      .clk_i   (clk_i),
      .rst_ni  (rst_ni),
      .req_i   (gnt_valid),
      .we_i    (gnt_req.we),
      .addr_i  (gnt_req.addr[SramAddrW-1:0]),
      .wdata_i (gnt_req.wdata),
      .wstrb_i (gnt_req.wstrb),
      .gnt_o   (),
      .rdata_o (sram_rdata_o),
      .rvalid_o(sram_rvalid_o)
  );

endmodule : soc_top
