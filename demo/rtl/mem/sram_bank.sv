// sram_bank.sv -- a bank of sram_wrapper instances behind one AXI4-Lite
// port, one clock-gated clk_gate per bank.
//
// The address space is split evenly across `NumBanks` banks: the top
// `BankIdxW` bits of the incoming `AddrWidth`-wide address pick the
// bank, the remaining low bits (`BankAddrWidth`) address a word inside
// it. Only the selected bank's clock toggles for a given access, which
// is the whole point of clk_gate.sv existing -- a design with one
// shared SRAM has nothing to gate.
//
// The AXI4-Lite port is flat signals rather than an `axi4_lite_if`
// port, because Icarus Verilog 13.0 does not support a module whose own
// port is interface-typed (`iverilog -g2012` on such a module: "syntax
// error" / "Errors in port declarations."). demo/verif/sram_bank_tb.sv
// wires an `axi4_lite_if` instance to these flat ports by name so the
// interface itself is still exercised end to end in real simulation;
// demo/verif/axi4_lite_monitor.sv is where the interface-typed port
// material lives instead.

module sram_bank #(
    parameter int unsigned NumBanks  = 4,
    // Total address width presented on the AXI4-Lite port; must be
    // wide enough to hold both the bank-select bits and an in-bank
    // offset (checked below).
    parameter int unsigned AddrWidth = 12
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic [         AddrWidth-1:0] aw_addr_i,
    input  logic                          aw_valid_i,
    output logic                          aw_ready_o,
    input  logic [soc_pkg::DataWidth-1:0] w_data_i,
    input  logic [soc_pkg::StrbWidth-1:0] w_strb_i,
    input  logic                          w_valid_i,
    output logic                          w_ready_o,
    output logic [                   1:0] b_resp_o,
    output logic                          b_valid_o,
    input  logic                          b_ready_i,
    input  logic [         AddrWidth-1:0] ar_addr_i,
    input  logic                          ar_valid_i,
    output logic                          ar_ready_o,
    output logic [soc_pkg::DataWidth-1:0] r_data_o,
    output logic [                   1:0] r_resp_o,
    output logic                          r_valid_o,
    input  logic                          r_ready_i
);

  localparam int unsigned BankIdxW = (NumBanks > 1) ? $clog2(NumBanks) : 1;
  // In-bank offset width: the low bits of AddrWidth left over once the
  // bank-select bits are taken off the top.
  localparam int unsigned BankAddrWidth = AddrWidth - BankIdxW;
  localparam int unsigned DataWidth = soc_pkg::DataWidth;
  localparam int unsigned StrbWidth = soc_pkg::StrbWidth;

  typedef enum logic [2:0] {
    StIdle,
    StWriteIssue,
    StWriteResp,
    StReadIssue,
    StReadWait,
    StReadResp
  } state_e;

  state_e state_q, state_d;

  logic [BankIdxW-1:0] wr_bank_q, wr_bank_d;
  logic [BankIdxW-1:0] rd_bank_q, rd_bank_d;
  logic [BankAddrWidth-1:0] wr_off_q, wr_off_d;
  logic [BankAddrWidth-1:0] rd_off_q, rd_off_d;
  logic [DataWidth-1:0] wr_data_q, wr_data_d;
  logic [StrbWidth-1:0] wr_strb_q, wr_strb_d;
  logic [DataWidth-1:0] rd_data_q;

  logic [NumBanks-1:0] bank_req;
  logic [NumBanks-1:0] bank_we;
  logic [BankAddrWidth-1:0] bank_addr[NumBanks];
  logic [DataWidth-1:0] bank_wdata[NumBanks];
  logic [StrbWidth-1:0] bank_wstrb[NumBanks];
  logic [NumBanks-1:0] bank_gnt;
  logic [DataWidth-1:0] bank_rdata[NumBanks];
  logic [NumBanks-1:0] bank_rvalid;

  for (genvar b = 0; b < NumBanks; b++) begin : g_bank
    logic gated_clk;

    // Gating only pays for itself once there is more than one bank to
    // pick between; a single-bank instance is always selected, so the
    // gate would just add a latch in series with the clock for nothing.
    if (NumBanks > 1) begin : g_gated_clk
      clk_gate u_clk_gate (
          .clk_i(clk_i),
          .rst_ni(rst_ni),
          .enable_i(bank_req[b]),
          .test_en_i(1'b0),
          .gated_clk_o(gated_clk)
      );
    end else begin : g_passthrough_clk
      assign gated_clk = clk_i;
    end

    sram_wrapper #(
        .AddrWidth(BankAddrWidth),
        .DataWidth(DataWidth),
        .Depth(1 << BankAddrWidth),
        .ReadLatency(1)
    ) u_sram (
        .clk_i(gated_clk),
        .rst_ni(rst_ni),
        .req_i(bank_req[b]),
        .we_i(bank_we[b]),
        .addr_i(bank_addr[b]),
        .wdata_i(bank_wdata[b]),
        .wstrb_i(bank_wstrb[b]),
        .gnt_o(bank_gnt[b]),
        .rdata_o(bank_rdata[b]),
        .rvalid_o(bank_rvalid[b])
    );
  end : g_bank

  always_comb begin
    bank_req = '0;
    bank_we  = '0;
    // A `'{default: '0}` aggregate assignment would be the idiomatic
    // way to clear these unpacked arrays, but Icarus Verilog 13.0
    // rejects it here ("syntax error" / "Malformed statement") --
    // dump-verified with a minimal repro -- so an explicit loop is used
    // instead.
    for (int i = 0; i < NumBanks; i++) begin
      bank_addr[i]  = '0;
      bank_wdata[i] = '0;
      bank_wstrb[i] = '0;
    end
    unique case (state_q)
      StWriteIssue: begin
        bank_req[wr_bank_q]   = 1'b1;
        bank_we[wr_bank_q]    = 1'b1;
        bank_addr[wr_bank_q]  = wr_off_q;
        bank_wdata[wr_bank_q] = wr_data_q;
        bank_wstrb[wr_bank_q] = wr_strb_q;
      end
      StReadIssue, StReadWait: begin
        bank_req[rd_bank_q]  = 1'b1;
        bank_addr[rd_bank_q] = rd_off_q;
      end
      default: ;
    endcase
  end

  // Single-outstanding: one AW+W pair or one AR is accepted at a time,
  // and AW/W are only accepted together (this slave never buffers one
  // half of a write waiting for the other).
  always_comb begin
    state_d   = state_q;
    wr_bank_d = wr_bank_q;
    wr_off_d  = wr_off_q;
    wr_data_d = wr_data_q;
    wr_strb_d = wr_strb_q;
    rd_bank_d = rd_bank_q;
    rd_off_d  = rd_off_q;
    // StWriteIssue/StReadIssue only advance once the selected bank
    // actually GRANTS the request -- this behavioural `sram_wrapper'
    // always grants immediately (`gnt_o = 1'b1'), so these two stay in
    // their Issue state for exactly one cycle today, but a real SRAM
    // with back-pressure (refresh cycle, ECC scrub, arbitration with a
    // second port) would need exactly this qualification, and without
    // it `bank_gnt' would be computed and never consumed.
    unique case (state_q)
      StIdle: begin
        if (aw_valid_i && w_valid_i) begin
          wr_bank_d = aw_addr_i[AddrWidth-1:BankAddrWidth];
          wr_off_d  = aw_addr_i[BankAddrWidth-1:0];
          wr_data_d = w_data_i;
          wr_strb_d = w_strb_i;
          state_d   = StWriteIssue;
        end else if (ar_valid_i) begin
          rd_bank_d = ar_addr_i[AddrWidth-1:BankAddrWidth];
          rd_off_d  = ar_addr_i[BankAddrWidth-1:0];
          state_d   = StReadIssue;
        end
      end
      StWriteIssue: if (bank_gnt[wr_bank_q]) state_d = StWriteResp;
      StWriteResp: if (b_ready_i) state_d = StIdle;
      StReadIssue: if (bank_gnt[rd_bank_q]) state_d = StReadWait;
      StReadWait: if (bank_rvalid[rd_bank_q]) state_d = StReadResp;
      StReadResp: if (r_ready_i) state_d = StIdle;
      default: state_d = StIdle;
    endcase
  end

  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      state_q   <= StIdle;
      wr_bank_q <= '0;
      wr_off_q  <= '0;
      wr_data_q <= '0;
      wr_strb_q <= '0;
      rd_bank_q <= '0;
      rd_off_q  <= '0;
      rd_data_q <= '0;
    end else begin
      state_q   <= state_d;
      wr_bank_q <= wr_bank_d;
      wr_off_q  <= wr_off_d;
      wr_data_q <= wr_data_d;
      wr_strb_q <= wr_strb_d;
      rd_bank_q <= rd_bank_d;
      rd_off_q  <= rd_off_d;
      if (state_q == StReadWait && bank_rvalid[rd_bank_q]) begin
        rd_data_q <= bank_rdata[rd_bank_q];
      end
    end
  end

  assign aw_ready_o = (state_q == StIdle) && w_valid_i;
  assign w_ready_o  = (state_q == StIdle) && aw_valid_i;
  assign b_resp_o   = 2'b00;
  assign b_valid_o  = (state_q == StWriteResp);

  assign ar_ready_o = (state_q == StIdle) && !(aw_valid_i && w_valid_i);
  assign r_resp_o   = 2'b00;
  assign r_valid_o  = (state_q == StReadResp);
  assign r_data_o   = rd_data_q;

`ifndef SOC_SVA_OFF
  // At most one bank may be driven in a given cycle -- a decode bug
  // that hits two banks at once would otherwise show up only as
  // corrupted data in whichever bank lost the race.
  a_bank_req_onehot :
  assert property (@(posedge clk_i) disable iff (!rst_ni) $onehot0(bank_req))
  else $error("sram_bank: more than one bank driven in the same cycle");

  // B can only be valid once this slave has actually issued a write to
  // a bank; catches a decode/FSM bug that fabricates a response.
  property p_b_after_write;
    @(posedge clk_i) disable iff (!rst_ni) b_valid_o |-> $past(
        state_q
    ) inside {StWriteIssue, StWriteResp};
  endproperty
  a_b_after_write :
  assert property (p_b_after_write)
  else $error("sram_bank: B response without a preceding accepted write");
`endif

endmodule : sram_bank
