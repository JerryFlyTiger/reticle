// axi4_lite_arbiter.sv -- round-robin arbiter over N request channels.
//
// Rotating priority rather than fixed, so a busy master 0 cannot starve
// master 3 -- the failure mode a fixed-priority arbiter has and which
// only shows up under load.

module axi4_lite_arbiter #(
    parameter int unsigned NumMasters = soc_pkg::NumMasters,
    parameter int unsigned IdxWidth   = (NumMasters > 1) ? $clog2(NumMasters) : 1
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic          [NumMasters-1:0] req_valid_i,
    output logic          [NumMasters-1:0] req_ready_o,
    input  soc_pkg::req_t                  req_i      [NumMasters],

    output logic                         gnt_valid_o,
    input  logic                         gnt_ready_i,
    output soc_pkg::req_t                gnt_req_o,
    output logic          [IdxWidth-1:0] gnt_idx_o
);

  logic [IdxWidth-1:0] rr_ptr_q, rr_ptr_d;
  logic [IdxWidth-1:0] winner;
  logic                any_req;

  assign any_req = |req_valid_i;

  // Walk the masters starting one past the last winner; the first
  // asserted request wins.
  always_comb begin
    winner = rr_ptr_q;
    for (int unsigned i = 0; i < NumMasters; i++) begin
      automatic int unsigned cand = (int'(rr_ptr_q) + int'(i)) % NumMasters;
      if (req_valid_i[cand]) begin
        winner = IdxWidth'(cand);
        break;
      end
    end
  end

  always_comb begin
    rr_ptr_d = rr_ptr_q;
    if (gnt_valid_o && gnt_ready_i) begin
      rr_ptr_d = (winner == IdxWidth'(NumMasters - 1)) ? '0 : (winner + 1'b1);
    end
  end

  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      rr_ptr_q <= '0;
    end else begin
      rr_ptr_q <= rr_ptr_d;
    end
  end

  always_comb begin
    req_ready_o = '0;
    if (any_req && gnt_ready_i) begin
      req_ready_o[winner] = 1'b1;
    end
  end

  assign gnt_valid_o = any_req;
  assign gnt_idx_o   = winner;
  assign gnt_req_o   = req_i[winner];

endmodule : axi4_lite_arbiter
