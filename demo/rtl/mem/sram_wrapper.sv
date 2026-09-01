// sram_wrapper.sv -- byte-enabled single-port SRAM behavioural wrapper.
//
// Real projects swap the `gen_behavioural' block for a vendor macro at
// synthesis time; the port list is what stays fixed, which is exactly
// why it is worth writing out in full.

module sram_wrapper #(
    parameter int unsigned AddrWidth   = 12,
    parameter int unsigned DataWidth   = soc_pkg::DataWidth,
    parameter int unsigned Depth       = 1 << AddrWidth,
    // Number of cycles between an accepted request and valid rdata.
    parameter int unsigned ReadLatency = 1
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic                          req_i,
    input  logic                          we_i,
    input  logic [         AddrWidth-1:0] addr_i,
    input  logic [         DataWidth-1:0] wdata_i,
    input  logic [soc_pkg::StrbWidth-1:0] wstrb_i,
    output logic                          gnt_o,

    output logic [DataWidth-1:0] rdata_o,
    output logic                 rvalid_o
);

  logic [DataWidth-1:0] mem[Depth];
  logic [DataWidth-1:0] rdata_q;
  logic [ReadLatency:0] rvalid_sr;
  logic [DataWidth-1:0] wmask;

  assign wmask = soc_pkg::strb_to_mask(wstrb_i);

  // No back-pressure in the behavioural model; a vendor macro that
  // needs it would drive this from its own busy flag.
  assign gnt_o = 1'b1;

  always_ff @(posedge clk_i) begin
    if (req_i && gnt_o) begin
      if (we_i) begin
        mem[addr_i] <= (mem[addr_i] & ~wmask) | (wdata_i & wmask);
      end else begin
        rdata_q <= mem[addr_i];
      end
    end
  end

  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      rvalid_sr <= '0;
    end else begin
      rvalid_sr <= {rvalid_sr[ReadLatency-1:0], req_i && gnt_o && !we_i};
    end
  end

  assign rvalid_o = rvalid_sr[ReadLatency];
  assign rdata_o  = rdata_q;

endmodule : sram_wrapper
