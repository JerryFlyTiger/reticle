// sram_bank_tb.sv -- self-checking testbench for sram_bank.sv.
//
// The interface itself (`axi4_lite_if`) is instantiated and driven for
// real: `u_bus`'s master-side signals are driven by the `axi_driver`
// program block, its slave-side signals by the `sram_bank` DUT, so the
// interface's signals genuinely carry the transactions even though
// neither the DUT's nor the driver's own port is interface-typed (see
// sram_bank.sv's header for why -- the same Icarus limitation applies
// here, so `axi_driver` takes flat ports and is wired to `u_bus.<name>`
// exactly the way the DUT is). `axi4_lite_monitor.sv` is the interface-
// typed-port half of this story, and it is verible-checked but never
// instantiated here because it cannot simulate under Icarus at all.

program automatic axi_driver #(
    parameter int unsigned AddrWidth = 12,
    parameter int unsigned DataWidth = soc_pkg::DataWidth,
    parameter int unsigned StrbWidth = soc_pkg::StrbWidth,
    parameter int unsigned NumBanks  = 4
) (
    input logic clk_i,
    input logic rst_ni,

    output logic [AddrWidth-1:0] aw_addr,
    output logic                 aw_valid,
    input  logic                 aw_ready,
    output logic [DataWidth-1:0] w_data,
    output logic [StrbWidth-1:0] w_strb,
    output logic                 w_valid,
    input  logic                 w_ready,
    input  logic [          1:0] b_resp,
    input  logic                 b_valid,
    output logic                 b_ready,
    output logic [AddrWidth-1:0] ar_addr,
    output logic                 ar_valid,
    input  logic                 ar_ready,
    input  logic [DataWidth-1:0] r_data,
    input  logic [          1:0] r_resp,
    input  logic                 r_valid,
    output logic                 r_ready
);

  import soc_verif_pkg::*;

  localparam int unsigned BankIdxW = (NumBanks > 1) ? $clog2(NumBanks) : 1;
  localparam int unsigned BankAddrWidth = AddrWidth - BankIdxW;
  localparam int unsigned WordsPerBank = 3;

  rw_checker chk;

  int unsigned b_acks;
  logic [DataWidth-1:0] rd_expected_q[$];
  logic [DataWidth-1:0] rd_observed_q[$];
  int unsigned rd_label_q[$];  // bank*1000+word, for readable failure messages only

  // Ordinary AXI master idiom: assert VALID and hold it until READY
  // arrives, however many cycles that takes -- NOT "assert VALID for
  // exactly one cycle, assuming the slave is already idle". The earlier
  // shape of this file did the latter, which meant every request was
  // issued only once the previous one's ENTIRE round trip (including
  // the DUT's own return to idle) had already completed, so
  // `aw_ready'/`ar_ready' were always already high the instant
  // `aw_valid'/`ar_valid' went high -- `axi4_lite_if.sv''s and
  // `axi4_lite_monitor.sv''s VALID-held-stable-until-READY properties
  // never had a real "VALID && !READY" cycle to check (measured: 0
  // such cycles for the whole run). Neither task below waits for the
  // matching RESPONSE before returning: the next request is issued
  // back to back, which is what actually makes `sram_bank' -- single-
  // outstanding, so it cannot accept a new request until it has
  // finished the previous one -- deassert READY while VALID is still
  // held, the real backpressure window those properties exist to
  // check (measured after this fix: 22 AW and 37 AR such cycles for
  // the same run). `do_write' doesn't need the response DATA (B
  // carries none), so a plain count of accepted B's is enough;
  // `do_read' pipelines the same way but needs each RDATA matched back
  // to the request that produced it, so expected values queue up in
  // issue order and a background process drains observed values in the
  // same (AXI4-Lite is in-order) order.
  task automatic do_write(input logic [AddrWidth-1:0] addr, input logic [DataWidth-1:0] data);
    aw_addr  = addr;
    w_data   = data;
    w_strb   = '1;
    aw_valid = 1'b1;
    w_valid  = 1'b1;
    @(posedge clk_i);
    while (!(aw_ready && w_ready)) @(posedge clk_i);
    aw_valid = 1'b0;
    w_valid  = 1'b0;
  endtask

  task automatic do_read(input logic [AddrWidth-1:0] addr, input logic [DataWidth-1:0] expected);
    ar_addr  = addr;
    ar_valid = 1'b1;
    @(posedge clk_i);
    while (!ar_ready) @(posedge clk_i);
    ar_valid = 1'b0;
    rd_expected_q.push_back(expected);
  endtask

  initial begin
    b_acks = 0;
    forever begin
      @(posedge clk_i);
      if (b_valid && b_ready) b_acks++;
      if (r_valid && r_ready) rd_observed_q.push_back(r_data);
    end
  end

  initial begin
    logic [DataWidth-1:0] expected;
    logic [DataWidth-1:0] observed;
    logic [AddrWidth-1:0] addr;
    bit ok;
    int unsigned bank;
    int unsigned word;
    int unsigned label;
    int unsigned total_words;

    total_words = NumBanks * WordsPerBank;
    chk = new();
    aw_addr = '0;
    ar_addr = '0;
    aw_valid = 1'b0;
    w_valid = 1'b0;
    ar_valid = 1'b0;
    b_ready = 1'b1;
    r_ready = 1'b1;
    w_data = '0;
    w_strb = '0;

    @(posedge rst_ni);
    @(posedge clk_i);

    // Write a few words into more than one bank before reading any of
    // them back, so a decode bug that clobbers a neighbouring bank
    // cannot hide behind read-after-write on the same bank. Issued
    // back to back (see do_write's own comment above), not one at a
    // time with a full round trip in between.
    for (bank = 0; bank < NumBanks; bank++) begin
      for (word = 0; word < WordsPerBank; word++) begin
        addr = {bank[BankIdxW-1:0], word[BankAddrWidth-1:0]};
        expected = 32'hA000_0000 | (bank << 16) | word;
        do_write(addr, expected);
      end
    end
    while (b_acks < total_words) @(posedge clk_i);

    for (bank = 0; bank < NumBanks; bank++) begin
      for (word = 0; word < WordsPerBank; word++) begin
        addr = {bank[BankIdxW-1:0], word[BankAddrWidth-1:0]};
        expected = 32'hA000_0000 | (bank << 16) | word;
        do_read(addr, expected);
        rd_label_q.push_back(bank * 1000 + word);
      end
    end
    while (rd_observed_q.size() < total_words) @(posedge clk_i);

    while (rd_expected_q.size() > 0) begin
      expected = rd_expected_q.pop_front();
      observed = rd_observed_q.pop_front();
      label    = rd_label_q.pop_front();
      bank     = label / 1000;
      word     = label % 1000;
      chk.set_expected(expected);
      ok = chk.check(observed);
      assert (ok)
      else
        $error(
            "sram_bank_tb: bank %0d word %0d: expected %h, got %h", bank, word, expected, observed
        );
    end

    if (chk.miss_count() == 0) begin
      $display("PASS: sram_bank %0d banks x %0d words (%s)", NumBanks, WordsPerBank, chk.summary());
    end else begin
      $display("FAIL: sram_bank %s", chk.summary());
      $fatal;
    end
    $finish;
  end

endprogram : axi_driver

module sram_bank_tb;

  localparam int unsigned AddrWidth = 12;
  localparam int unsigned NumBanks  = 4;

  logic clk_i = 1'b0;
  logic rst_ni = 1'b0;

  always #5 clk_i = ~clk_i;

  initial begin
    rst_ni = 1'b0;
    #12;
    rst_ni = 1'b1;
  end

  axi4_lite_if #(
      .AddrWidth(AddrWidth),
      .DataWidth(soc_pkg::DataWidth)
  ) u_bus (
      .clk_i (clk_i),
      .rst_ni(rst_ni)
  );

  sram_bank #(
      .NumBanks (NumBanks),
      .AddrWidth(AddrWidth)
  ) dut (
      .clk_i     (clk_i),
      .rst_ni    (rst_ni),
      .aw_addr_i (u_bus.aw_addr),
      .aw_valid_i(u_bus.aw_valid),
      .aw_ready_o(u_bus.aw_ready),
      .w_data_i  (u_bus.w_data),
      .w_strb_i  (u_bus.w_strb),
      .w_valid_i (u_bus.w_valid),
      .w_ready_o (u_bus.w_ready),
      .b_resp_o  (u_bus.b_resp),
      .b_valid_o (u_bus.b_valid),
      .b_ready_i (u_bus.b_ready),
      .ar_addr_i (u_bus.ar_addr),
      .ar_valid_i(u_bus.ar_valid),
      .ar_ready_o(u_bus.ar_ready),
      .r_data_o  (u_bus.r_data),
      .r_resp_o  (u_bus.r_resp),
      .r_valid_o (u_bus.r_valid),
      .r_ready_i (u_bus.r_ready)
  );

  axi_driver #(
      .AddrWidth(AddrWidth),
      .DataWidth(soc_pkg::DataWidth),
      .StrbWidth(soc_pkg::StrbWidth),
      .NumBanks (NumBanks)
  ) u_drv (
      .clk_i   (clk_i),
      .rst_ni  (rst_ni),
      .aw_addr (u_bus.aw_addr),
      .aw_valid(u_bus.aw_valid),
      .aw_ready(u_bus.aw_ready),
      .w_data  (u_bus.w_data),
      .w_strb  (u_bus.w_strb),
      .w_valid (u_bus.w_valid),
      .w_ready (u_bus.w_ready),
      .b_resp  (u_bus.b_resp),
      .b_valid (u_bus.b_valid),
      .b_ready (u_bus.b_ready),
      .ar_addr (u_bus.ar_addr),
      .ar_valid(u_bus.ar_valid),
      .ar_ready(u_bus.ar_ready),
      .r_data  (u_bus.r_data),
      .r_resp  (u_bus.r_resp),
      .r_valid (u_bus.r_valid),
      .r_ready (u_bus.r_ready)
  );

`ifndef SOC_COVERAGE_OFF
  // Two independent things worth knowing were actually hit during this
  // run: which bank a write landed in, and whether AW/AR ever competed
  // for the slave in the same cycle (this slave never buffers one, so
  // seeing both bins hit is what confirms the single-outstanding
  // arbitration in sram_bank.sv was genuinely exercised both ways).
  covergroup cg_axi_bank @(posedge clk_i);
    cp_wr_bank: coverpoint u_bus.aw_addr[AddrWidth-1] iff (u_bus.aw_valid && u_bus.aw_ready) {
      bins bank0 = {1'b0}; bins bank1 = {1'b1};
    }
    cp_channel_activity: coverpoint {
      u_bus.aw_valid && u_bus.aw_ready, u_bus.ar_valid && u_bus.ar_ready
    } {
      bins write_accepted = {2'b10}; bins read_accepted = {2'b01}; bins idle = {2'b00};
    }
  endgroup : cg_axi_bank

  cg_axi_bank cg = new();
`endif

endmodule : sram_bank_tb
