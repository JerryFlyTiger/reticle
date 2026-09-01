#!/usr/bin/env python3
"""Build an "industrial-scale" SystemVerilog tree, used to probe verible's
references.

Not for show -- it's to give references a ground truth to check against:
the generator records the (file, line, col) of every instantiation itself,
writing it out as truth.json, so the LSP's returned list can later be
checked against it.

The shape deliberately mirrors real RTL practice:
  - parameterized modules, long names
  - named instantiations with 40+ ports
  - a leaf instantiated dozens of times (references' main use case:
    "who uses this module")
  - nested directories + verible.filelist

**Why this is checked into the repo instead of being a throwaway**: same
reason as `dev/lsp-probe.py` -- the friction of regenerating material every
time tempts you to skip the actual measurement step. And without it,
M59's record of "890 files / 154k lines, 400 instantiations, 400/400
correct" would be a claim nobody could reproduce. `demo/rtl/` is a 6-file
showcase and can't surface scale problems; this script fills in the scale
end, and the two don't overlap.

**Do not use this as showcase material.** The generated code is a
mechanically repetitive skeleton with no semantic meaning, and putting it
into `demo/` would zero out that directory's credibility. Point the output
directory at a scratch location outside the repo.

Usage (the two sizes used for M59's real measurements):
    dev/gen-big-rtl.py /tmp/bigrtl                      # default 186 files / 31k lines
    # 890 files / 154k lines: change N_BLOCKS to 800, N_CLUSTERS to 80

    dev/lsp-probe.py --server verible-verilog-ls --root /tmp/bigrtl \\
        --file /tmp/bigrtl/leaf/alu_execute_core.sv \\
        --method textDocument/references --line 1 --char 7

Every item returned can be checked one by one against `truth.json`'s
`instantiations` (0-based line/col, matching LSP). During M59 the results
matched completely, with zero false positives.
"""
import json
import os
import sys

ROOT = sys.argv[1]

N_BLOCKS = 160
N_CLUSTERS = 16
BLOCKS_PER_CLUSTER = 10

LEAVES = [
    "sync_fifo_dualclk",
    "round_robin_arbiter",
    "cdc_bit_synchronizer",
    "alu_execute_core",
    "register_file_bank",
    "ecc_secded_encoder",
    "ecc_secded_decoder",
    "axi_skid_buffer",
]

truth = {"instantiations": {}, "files": []}


def emit(relpath, text):
    path = os.path.join(ROOT, relpath)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    truth["files"].append(relpath)


# ---------------------------------------------------------------- package
pkg = """// big_pkg.sv -- shared parameters and types for the generated tree.
package big_pkg;

  parameter int unsigned DataWidth    = 64;
  parameter int unsigned AddrWidth    = 40;
  parameter int unsigned IdWidth      = 8;
  parameter int unsigned NumLanes     = 4;
  parameter int unsigned EccDataWidth = 32;
  parameter int unsigned EccCodeWidth = 7;

  typedef enum logic [3:0] {
    OpAdd,
    OpSub,
    OpAnd,
    OpOr,
    OpXor,
    OpShiftLeft,
    OpShiftRight,
    OpCompare
  } exec_op_e;

  typedef struct packed {
    logic [AddrWidth-1:0] addr;
    logic [IdWidth-1:0]   id;
    logic [7:0]           len;
    logic                 write;
  } req_header_t;

endpackage : big_pkg
"""
emit("pkg/big_pkg.sv", pkg)


# ------------------------------------------------------------------ leaves
def leaf_src(name):
    return f"""// {name}.sv -- generated leaf module.
module {name} #(
    parameter int unsigned DataWidth = big_pkg::DataWidth,
    parameter int unsigned Depth     = 8,
    parameter bit          Registered = 1'b1
) (
    input  logic                 clk_i,
    input  logic                 rst_ni,
    input  logic                 valid_i,
    output logic                 ready_o,
    input  logic [DataWidth-1:0] data_i,
    output logic                 valid_o,
    input  logic                 ready_i,
    output logic [DataWidth-1:0] data_o,
    output logic                 error_o
);

  localparam int unsigned PtrWidth = $clog2(Depth);

  logic [DataWidth-1:0] stage_q;
  logic                 stage_valid_q;
  logic [PtrWidth-1:0]  count_q;

  assign ready_o = ~stage_valid_q | ready_i;
  assign data_o  = Registered ? stage_q : data_i;
  assign valid_o = Registered ? stage_valid_q : valid_i;
  assign error_o = (count_q == '1) & valid_i & ~ready_o;

  always_ff @(posedge clk_i or negedge rst_ni) begin
    if (!rst_ni) begin
      stage_q       <= '0;
      stage_valid_q <= 1'b0;
      count_q       <= '0;
    end else begin
      if (valid_i && ready_o) begin
        stage_q       <= data_i;
        stage_valid_q <= 1'b1;
        count_q       <= count_q + 1'b1;
      end else if (ready_i) begin
        stage_valid_q <= 1'b0;
      end
    end
  end

endmodule : {name}
"""


for leaf in LEAVES:
    emit(f"leaf/{leaf}.sv", leaf_src(leaf))


# ------------------------------------------------------------------ blocks
# Each block has 45 ports (40+ port instantiations are common in industry
# RTL), instantiating 4 leaves internally.
def block_ports():
    lines = []
    lines.append("    input  logic clk_i,")
    lines.append("    input  logic rst_ni,")
    for lane in range(4):
        lines.append(f"    input  logic                 lane{lane}_valid_i,")
        lines.append(f"    output logic                 lane{lane}_ready_o,")
        lines.append(f"    input  logic [DataWidth-1:0] lane{lane}_data_i,")
        lines.append(f"    output logic                 lane{lane}_valid_o,")
        lines.append(f"    input  logic                 lane{lane}_ready_i,")
        lines.append(f"    output logic [DataWidth-1:0] lane{lane}_data_o,")
        lines.append(f"    output logic                 lane{lane}_error_o,")
    lines.append("    input  big_pkg::req_header_t header_i,")
    lines.append("    input  big_pkg::exec_op_e    op_i,")
    for extra in range(12):
        lines.append(f"    input  logic [DataWidth-1:0] cfg{extra}_i,")
    lines.append("    output logic                 status_valid_o,")
    lines.append("    output logic [DataWidth-1:0] status_data_o")
    return lines


def block_src(idx, name):
    leaves_used = [LEAVES[(idx + k) % len(LEAVES)] for k in range(4)]
    lines = []
    lines.append(f"// {name}.sv -- generated block module (45 ports, 4 leaves).")
    lines.append(f"module {name} #(")
    lines.append("    parameter int unsigned DataWidth = big_pkg::DataWidth,")
    lines.append(f"    parameter int unsigned FifoDepth = {8 + (idx % 8) * 2},")
    lines.append("    parameter bit          EnableEcc = 1'b1")
    lines.append(") (")
    lines.extend(block_ports())
    lines.append(");")
    lines.append("")
    lines.append("  logic [DataWidth-1:0] internal_data [4];")
    lines.append("  logic                 internal_error[4];")
    lines.append("")
    for lane, leaf in enumerate(leaves_used):
        lines.append(f"  {leaf} #(")
        lines.append("      .DataWidth (DataWidth),")
        lines.append("      .Depth     (FifoDepth),")
        lines.append("      .Registered(1'b1)")
        lines.append(f"  ) u_lane{lane}_{leaf} (")
        lines.append("      .clk_i  (clk_i),")
        lines.append("      .rst_ni (rst_ni),")
        lines.append(f"      .valid_i(lane{lane}_valid_i),")
        lines.append(f"      .ready_o(lane{lane}_ready_o),")
        lines.append(f"      .data_i (lane{lane}_data_i),")
        lines.append(f"      .valid_o(lane{lane}_valid_o),")
        lines.append(f"      .ready_i(lane{lane}_ready_i),")
        lines.append(f"      .data_o (internal_data[{lane}]),")
        lines.append(f"      .error_o(internal_error[{lane}])")
        lines.append("  );")
        lines.append("")
        lines.append(f"  assign lane{lane}_data_o  = internal_data[{lane}];")
        lines.append(f"  assign lane{lane}_error_o = internal_error[{lane}];")
        lines.append("")
    lines.append("  assign status_valid_o = lane0_valid_o & lane1_valid_o;")
    lines.append("  assign status_data_o  = internal_data[0] ^ cfg0_i ^ cfg1_i;")
    lines.append("")
    lines.append(f"endmodule : {name}")
    text = "\n".join(lines) + "\n"
    return f"blk/{name}.sv", lines, text


block_names = []
for idx in range(N_BLOCKS):
    name = f"datapath_block_{idx:03d}"
    block_names.append(name)
    relpath, lines, text = block_src(idx, name)
    emit(relpath, text)
    # Record the position of every leaf instantiation in this file (the
    # module name appears at the start of the instantiation)
    for i, line in enumerate(lines):
        stripped = line.strip()
        for leaf in LEAVES:
            if stripped.startswith(leaf + " #("):
                truth["instantiations"].setdefault(leaf, []).append(
                    {"file": relpath, "line": i, "char": line.find(leaf)}
                )


# ---------------------------------------------------------------- clusters
def cluster_src(name, members):
    lines = []
    lines.append(f"// {name}.sv -- generated cluster ({len(members)} blocks).")
    lines.append(f"module {name} #(")
    lines.append("    parameter int unsigned DataWidth = big_pkg::DataWidth")
    lines.append(") (")
    lines.append("    input  logic clk_i,")
    lines.append("    input  logic rst_ni,")
    lines.append("    input  big_pkg::req_header_t header_i,")
    lines.append("    input  big_pkg::exec_op_e    op_i,")
    lines.append("    input  logic [DataWidth-1:0] cluster_data_i,")
    lines.append("    output logic [DataWidth-1:0] cluster_data_o,")
    lines.append("    output logic                 cluster_valid_o")
    lines.append(");")
    lines.append("")
    lines.append(f"  logic [DataWidth-1:0] blk_data [{len(members)}];")
    lines.append(f"  logic                 blk_valid[{len(members)}];")
    lines.append("")
    for bi, bname in enumerate(members):
        lines.append(f"  {bname} #(")
        lines.append("      .DataWidth(DataWidth),")
        lines.append("      .EnableEcc(1'b1)")
        lines.append(f"  ) u_{bname} (")
        lines.append("      .clk_i (clk_i),")
        lines.append("      .rst_ni(rst_ni),")
        for lane in range(4):
            lines.append(f"      .lane{lane}_valid_i(1'b1),")
            lines.append(f"      .lane{lane}_ready_o(),")
            lines.append(f"      .lane{lane}_data_i (cluster_data_i),")
            lines.append(f"      .lane{lane}_ready_i(1'b1),")
            if lane == 0:
                lines.append(f"      .lane{lane}_valid_o(blk_valid[{bi}]),")
                lines.append(f"      .lane{lane}_data_o (blk_data[{bi}]),")
            else:
                lines.append(f"      .lane{lane}_valid_o(),")
                lines.append(f"      .lane{lane}_data_o (),")
            lines.append(f"      .lane{lane}_error_o(),")
        lines.append("      .header_i(header_i),")
        lines.append("      .op_i    (op_i),")
        for extra in range(12):
            lines.append(f"      .cfg{extra}_i(cluster_data_i),")
        lines.append("      .status_valid_o(),")
        lines.append("      .status_data_o ()")
        lines.append("  );")
        lines.append("")
    lines.append("  assign cluster_data_o  = blk_data[0];")
    lines.append("  assign cluster_valid_o = blk_valid[0];")
    lines.append("")
    lines.append(f"endmodule : {name}")
    return "\n".join(lines) + "\n", lines


cluster_names = []
for cidx in range(N_CLUSTERS):
    name = f"compute_cluster_{cidx:02d}"
    cluster_names.append(name)
    members = block_names[
        cidx * BLOCKS_PER_CLUSTER : (cidx + 1) * BLOCKS_PER_CLUSTER
    ]
    text, lines = cluster_src(name, members)
    relpath = f"cluster/{name}.sv"
    emit(relpath, text)
    for i, line in enumerate(lines):
        stripped = line.strip()
        for bname in members:
            if stripped.startswith(bname + " #("):
                truth["instantiations"].setdefault(bname, []).append(
                    {"file": relpath, "line": i, "char": line.find(bname)}
                )

# --------------------------------------------------------------------- top
lines = []
lines.append("// big_top.sv -- generated top level.")
lines.append("module big_top #(")
lines.append("    parameter int unsigned DataWidth = big_pkg::DataWidth")
lines.append(") (")
lines.append("    input  logic clk_i,")
lines.append("    input  logic rst_ni,")
lines.append("    input  big_pkg::req_header_t header_i,")
lines.append("    input  big_pkg::exec_op_e    op_i,")
lines.append("    input  logic [DataWidth-1:0] top_data_i,")
lines.append("    output logic [DataWidth-1:0] top_data_o")
lines.append(");")
lines.append("")
lines.append(f"  logic [DataWidth-1:0] cluster_data[{N_CLUSTERS}];")
lines.append(f"  logic                 cluster_valid[{N_CLUSTERS}];")
lines.append("")
for cidx, cname in enumerate(cluster_names):
    lines.append(f"  {cname} #(")
    lines.append("      .DataWidth(DataWidth)")
    lines.append(f"  ) u_{cname} (")
    lines.append("      .clk_i         (clk_i),")
    lines.append("      .rst_ni        (rst_ni),")
    lines.append("      .header_i      (header_i),")
    lines.append("      .op_i          (op_i),")
    lines.append("      .cluster_data_i(top_data_i),")
    lines.append(f"      .cluster_data_o(cluster_data[{cidx}]),")
    lines.append(f"      .cluster_valid_o(cluster_valid[{cidx}])")
    lines.append("  );")
    lines.append("")
lines.append("  assign top_data_o = cluster_data[0];")
lines.append("")
lines.append("endmodule : big_top")
top_text = "\n".join(lines) + "\n"
emit("top/big_top.sv", top_text)
for i, line in enumerate(lines):
    stripped = line.strip()
    for cname in cluster_names:
        if stripped.startswith(cname + " #("):
            truth["instantiations"].setdefault(cname, []).append(
                {"file": "top/big_top.sv", "line": i, "char": line.find(cname)}
            )

# --------------------------------------------------------------- filelist
order = (
    ["pkg/big_pkg.sv"]
    + [f"leaf/{n}.sv" for n in LEAVES]
    + [f"blk/{n}.sv" for n in block_names]
    + [f"cluster/{n}.sv" for n in cluster_names]
    + ["top/big_top.sv"]
)
with open(os.path.join(ROOT, "verible.filelist"), "w") as f:
    f.write("\n".join(order) + "\n")

with open(os.path.join(ROOT, "truth.json"), "w") as f:
    json.dump(truth, f, indent=1)

total_lines = 0
for rel in truth["files"]:
    with open(os.path.join(ROOT, rel)) as f:
        total_lines += sum(1 for _ in f)

print(f"files      : {len(truth['files'])}")
print(f"total lines: {total_lines}")
for mod in ["alu_execute_core", "datapath_block_000", "compute_cluster_00"]:
    print(f"{mod:22s}: {len(truth['instantiations'].get(mod, []))} instantiation(s)")
