// sram_bank_4k.sv -- sram_bank pinned to NumBanks=4 within a 4KB
// (AddrWidth=12) address space, a parameter-fixing wrapper.
//
// A parameter-fixing wrapper is the one place `.*` is defensible in real
// RTL: this module's own port list is declared to match `sram_bank`'s
// exactly, so the wildcard connects every port by construction rather than
// by coincidence. Like `core/status_regs_stub.sv`, this module exists as
// material and is deliberately not wired into `top/soc_top.sv`.

module sram_bank_4k (
    input logic clk_i,
    input logic rst_ni,

    input  logic [                  11:0] aw_addr_i,
    input  logic                          aw_valid_i,
    output logic                          aw_ready_o,
    input  logic [soc_pkg::DataWidth-1:0] w_data_i,
    input  logic [soc_pkg::StrbWidth-1:0] w_strb_i,
    input  logic                          w_valid_i,
    output logic                          w_ready_o,
    output logic [                   1:0] b_resp_o,
    output logic                          b_valid_o,
    input  logic                          b_ready_i,
    input  logic [                  11:0] ar_addr_i,
    input  logic                          ar_valid_i,
    output logic                          ar_ready_o,
    output logic [soc_pkg::DataWidth-1:0] r_data_o,
    output logic [                   1:0] r_resp_o,
    output logic                          r_valid_o,
    input  logic                          r_ready_i
);

  sram_bank #(
      .NumBanks (4),
      .AddrWidth(12)
  ) u_bank (
      .*
  );

endmodule : sram_bank_4k
