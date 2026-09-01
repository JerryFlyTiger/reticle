// soc_pkg.sv -- shared parameters, enums and structs for the demo SoC.
//
// Everything the modules below agree on lives here rather than in
// `define macros, so that a tool reading one file at a time still sees
// real types.

package soc_pkg;

  parameter int unsigned AddrWidth  = 32;
  parameter int unsigned DataWidth  = 32;
  parameter int unsigned StrbWidth  = DataWidth / 8;
  parameter int unsigned NumMasters = 4;
  parameter int unsigned RegCount   = 32;
  parameter int unsigned RegIdxW    = $clog2(RegCount);

  // ALU operation encoding. Deliberately RISC-V shaped so the opcodes
  // are recognisable rather than invented.
  typedef enum logic [3:0] {
    AluAdd  = 4'h0,
    AluSub  = 4'h1,
    AluAnd  = 4'h2,
    AluOr   = 4'h3,
    AluXor  = 4'h4,
    AluSll  = 4'h5,
    AluSrl  = 4'h6,
    AluSra  = 4'h7,
    AluSlt  = 4'h8,
    AluSltu = 4'h9
  } alu_op_e;

  // A single bus request, as seen by the arbiter and the memory.
  typedef struct packed {
    logic [AddrWidth-1:0] addr;
    logic [DataWidth-1:0] wdata;
    logic [StrbWidth-1:0] wstrb;
    logic                 we;
  } req_t;

  typedef struct packed {
    logic [DataWidth-1:0] rdata;
    logic                 error;
  } rsp_t;

  // Byte-strobe to bit-mask expansion, used by every write path.
  function automatic logic [DataWidth-1:0] strb_to_mask(logic [StrbWidth-1:0] strb);
    logic [DataWidth-1:0] mask;
    mask = '0;
    for (int unsigned i = 0; i < StrbWidth; i++) begin
      mask[i*8+:8] = {8{strb[i]}};
    end
    return mask;
  endfunction

endpackage : soc_pkg
