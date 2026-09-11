// sram_dual_channel.sv -- two independent SRAM banks behind one shared
// clock/reset, each wired through `sram_wrapper' via AUTOINST/AUTO_TEMPLATE.
//
// This is the showcase file for M127's AUTO_TEMPLATE `@' instance-number
// substitution and `[]' bit-range tokens: ONE template body, applied to
// TWO instances (`u_ch0'/`u_ch1'), and every templated connection carries
// its own `// Templated' annotation. `req_i'/`we_i'/`gnt_o'/`rvalid_o' are
// 1-bit (so `[]' resolves to nothing); `addr_i'/`wdata_i'/`wstrb_i'/
// `rdata_o' are each a differently-parameterised range (so `[]' resolves
// to that port's own declared width) -- one template, both port shapes.
// `clk_i'/`rst_ni' are pinned by an EXACT rule that must win over the
// wildcard rule below it (both end in `_i'; without the exact rule,
// `clk_i' would be wrongly renamed to `clk_ch@_i').

module sram_dual_channel #(
    parameter int unsigned AddrWidth = 12,
    parameter int unsigned DataWidth = soc_pkg::DataWidth
) (
    input logic clk_i,
    input logic rst_ni,

    input  logic                          req_ch0_i,
    input  logic                          we_ch0_i,
    input  logic [         AddrWidth-1:0] addr_ch0_i,
    input  logic [         DataWidth-1:0] wdata_ch0_i,
    input  logic [soc_pkg::StrbWidth-1:0] wstrb_ch0_i,
    output logic                          gnt_ch0_o,
    output logic [         DataWidth-1:0] rdata_ch0_o,
    output logic                          rvalid_ch0_o,

    input  logic                          req_ch1_i,
    input  logic                          we_ch1_i,
    input  logic [         AddrWidth-1:0] addr_ch1_i,
    input  logic [         DataWidth-1:0] wdata_ch1_i,
    input  logic [soc_pkg::StrbWidth-1:0] wstrb_ch1_i,
    output logic                          gnt_ch1_o,
    output logic [         DataWidth-1:0] rdata_ch1_o,
    output logic                          rvalid_ch1_o
);

  /* sram_wrapper AUTO_TEMPLATE (
     .clk_i (clk_i),
     .rst_ni(rst_ni),
     .\(.*\)_i (\1_ch@_i[]),
     .\(.*\)_o (\1_ch@_o[]),
     ); */
  sram_wrapper #(
      .AddrWidth(AddrWidth),
      .DataWidth(DataWidth)
  ) u_ch0 (  /*AUTOINST*/
      // Outputs
      .gnt_o   (gnt_ch0_o),                           // Templated
      .rdata_o (rdata_ch0_o[DataWidth-1:0]),          // Templated
      .rvalid_o(rvalid_ch0_o),                        // Templated
      // Inputs
      .clk_i   (clk_i),                               // Templated
      .rst_ni  (rst_ni),                              // Templated
      .req_i   (req_ch0_i),                           // Templated
      .we_i    (we_ch0_i),                            // Templated
      .addr_i  (addr_ch0_i[AddrWidth-1:0]),           // Templated
      .wdata_i (wdata_ch0_i[DataWidth-1:0]),          // Templated
      .wstrb_i (wstrb_ch0_i[soc_pkg::StrbWidth-1:0])
  );  // Templated

  sram_wrapper #(
      .AddrWidth(AddrWidth),
      .DataWidth(DataWidth)
  ) u_ch1 (  /*AUTOINST*/
      // Outputs
      .gnt_o   (gnt_ch1_o),                           // Templated
      .rdata_o (rdata_ch1_o[DataWidth-1:0]),          // Templated
      .rvalid_o(rvalid_ch1_o),                        // Templated
      // Inputs
      .clk_i   (clk_i),                               // Templated
      .rst_ni  (rst_ni),                              // Templated
      .req_i   (req_ch1_i),                           // Templated
      .we_i    (we_ch1_i),                            // Templated
      .addr_i  (addr_ch1_i[AddrWidth-1:0]),           // Templated
      .wdata_i (wdata_ch1_i[DataWidth-1:0]),          // Templated
      .wstrb_i (wstrb_ch1_i[soc_pkg::StrbWidth-1:0])
  );  // Templated

endmodule : sram_dual_channel
