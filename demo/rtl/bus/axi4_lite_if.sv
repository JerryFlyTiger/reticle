// axi4_lite_if.sv -- AXI4-Lite bus bundled as a SystemVerilog interface.
//
// axi4_lite_arbiter.sv predates this file and still carries its channel
// signals as flat ports; a real design settles on one style project-wide,
// but a single bundled interface (with `master`/`slave`/`monitor` modports)
// is what a memory-mapped slave like sram_bank.sv actually wants to plug
// into, so it is added here rather than retrofitted onto the arbiter.

interface axi4_lite_if #(
    parameter int unsigned AddrWidth = soc_pkg::AddrWidth,
    parameter int unsigned DataWidth = soc_pkg::DataWidth
) (
    input logic clk_i,
    input logic rst_ni
);

  localparam int unsigned StrbWidth = DataWidth / 8;

  logic [AddrWidth-1:0] aw_addr;
  logic aw_valid;
  logic aw_ready;

  logic [DataWidth-1:0] w_data;
  logic [StrbWidth-1:0] w_strb;
  logic w_valid;
  logic w_ready;

  logic [1:0] b_resp;
  logic b_valid;
  logic b_ready;

  logic [AddrWidth-1:0] ar_addr;
  logic ar_valid;
  logic ar_ready;

  logic [DataWidth-1:0] r_data;
  logic [1:0] r_resp;
  logic r_valid;
  logic r_ready;

  modport master(
      output aw_addr, aw_valid,
      input aw_ready,
      output w_data, w_strb, w_valid,
      input w_ready,
      input b_resp, b_valid,
      output b_ready,
      output ar_addr, ar_valid,
      input ar_ready,
      input r_data, r_resp, r_valid,
      output r_ready
  );

  modport slave(
      input aw_addr, aw_valid,
      output aw_ready,
      input w_data, w_strb, w_valid,
      output w_ready,
      output b_resp, b_valid,
      input b_ready,
      input ar_addr, ar_valid,
      output ar_ready,
      output r_data, r_resp, r_valid,
      input r_ready
  );

  modport monitor(
      input aw_addr, aw_valid, aw_ready,
      input w_data, w_strb, w_valid, w_ready,
      input b_resp, b_valid, b_ready,
      input ar_addr, ar_valid, ar_ready,
      input r_data, r_resp, r_valid, r_ready
  );

`ifndef SOC_SVA_OFF
  // Once a channel's VALID is asserted it must stay asserted, with its
  // payload held stable, until the matching READY arrives -- the
  // handshake rule every AXI4-Lite channel shares.
  property p_aw_stable;
    @(posedge clk_i) disable iff (!rst_ni) (aw_valid && !aw_ready) |=> (aw_valid && $stable(
        aw_addr
    ));
  endproperty
  a_aw_stable :
  assert property (p_aw_stable)
  else $error("axi4_lite_if: AW channel dropped VALID or changed ADDR before READY");

  property p_ar_stable;
    @(posedge clk_i) disable iff (!rst_ni) (ar_valid && !ar_ready) |=> (ar_valid && $stable(
        ar_addr
    ));
  endproperty
  a_ar_stable :
  assert property (p_ar_stable)
  else $error("axi4_lite_if: AR channel dropped VALID or changed ADDR before READY");
`endif

endinterface : axi4_lite_if
