// regfile.sv -- two-read one-write register file.
//
// Register 0 reads as zero and ignores writes, the usual RISC-style
// convention; keeping that here rather than in the caller means every
// user of this file gets it for free.

module regfile #(
    parameter int unsigned DataWidth  = soc_pkg::DataWidth,
    parameter int unsigned NumRegs    = soc_pkg::RegCount,
    // Read-during-write on the same address returns the NEW value when
    // set, the stored one otherwise.
    parameter bit          WriteFirst = 1'b1
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic [$clog2(NumRegs)-1:0] raddr_a_i,
    output logic [      DataWidth-1:0] rdata_a_o,

    input  logic [$clog2(NumRegs)-1:0] raddr_b_i,
    output logic [      DataWidth-1:0] rdata_b_o,

    input logic                       we_i,
    input logic [$clog2(NumRegs)-1:0] waddr_i,
    input logic [      DataWidth-1:0] wdata_i
);

  logic [DataWidth-1:0] mem      [NumRegs];
  logic                 write_en;

  assign write_en = we_i && (waddr_i != '0);

  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      for (int unsigned i = 0; i < NumRegs; i++) begin
        mem[i] <= '0;
      end
    end else if (write_en) begin
      mem[waddr_i] <= wdata_i;
    end
  end

  function automatic logic [DataWidth-1:0] read_port(logic [$clog2(NumRegs)-1:0] addr);
    if (addr == '0) begin
      return '0;
    end
    if (WriteFirst && write_en && (addr == waddr_i)) begin
      return wdata_i;
    end
    return mem[addr];
  endfunction

  assign rdata_a_o = read_port(raddr_a_i);
  assign rdata_b_o = read_port(raddr_b_i);

endmodule : regfile
