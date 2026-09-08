// axi4_lite_monitor.sv -- passive AXI4-Lite protocol checker, driven by
// binding onto an axi4_lite_if instance.
//
// This is the one place in the whole demo where a module's own port is
// interface-typed (`axi4_lite_if.monitor bus`). It cannot be simulated
// with Icarus Verilog 13.0 for two independent reasons, either one of
// which is fatal on its own: an interface-typed module port is a
// syntax error ("Errors in port declarations."), and every concurrent
// assertion below is one Icarus rejects too ("syntax error" /
// "Invalid module item" -- see sram_bank.sv's `ifndef SOC_SVA_OFF`
// guard, which exists because of the second problem alone). A guard
// here would buy nothing since neither half of this file can run under
// Icarus regardless, so demo/tools/run_sim.sh simply never compiles it;
// demo/README.md names it explicitly as checked by verible but never
// simulated. Both limitations are Icarus's, not the language's or the
// design's: the material below is IEEE 1800-2017 legal, and it passes
// verible-verilog-lint's full default rule set with no waivers.

module axi4_lite_monitor (
    axi4_lite_if.monitor bus
);

  // Once a channel's VALID is asserted it must stay asserted, with its
  // payload held stable, until the matching READY arrives.
  property p_aw_stable;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni)
    (bus.aw_valid && !bus.aw_ready) |=> (bus.aw_valid && $stable(
        bus.aw_addr
    ));
  endproperty
  a_aw_stable :
  assert property (p_aw_stable)
  else $error("axi4_lite_monitor: AW channel dropped VALID or changed ADDR before READY");

  property p_w_stable;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni)
    (bus.w_valid && !bus.w_ready) |=> (bus.w_valid && $stable(
        bus.w_data
    ) && $stable(
        bus.w_strb
    ));
  endproperty
  a_w_stable :
  assert property (p_w_stable)
  else $error("axi4_lite_monitor: W channel dropped VALID or changed DATA/STRB before READY");

  property p_ar_stable;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni)
    (bus.ar_valid && !bus.ar_ready) |=> (bus.ar_valid && $stable(
        bus.ar_addr
    ));
  endproperty
  a_ar_stable :
  assert property (p_ar_stable)
  else $error("axi4_lite_monitor: AR channel dropped VALID or changed ADDR before READY");

  // A response channel must never fire without a matching request
  // channel having been accepted first: a fabricated B or R is exactly
  // the kind of bug a passive monitor exists to catch, because it never
  // shows up as a functional mismatch on the requester side alone.
  //
  // A bare `$past(aw_valid && aw_ready)' (one-cycle-earlier check) is
  // WRONG here and was the actual shape of an earlier, buggy version of
  // this file: traced against the real `sram_bank' DUT, an accepted
  // AW+W produces `b_valid' TWO cycles later (StWriteIssue ->
  // StWriteResp), and an accepted AR produces `r_valid' THREE TO FOUR
  // cycles later (StReadIssue -> StReadWait, possibly held one or more
  // extra cycles -> StReadResp) -- under any SVA-capable simulator, a
  // one-cycle-only check fails on every legitimate transaction. A
  // monitor watching only the interface (no visibility into the DUT's
  // internal FSM or its exact latency) cannot assume any FIXED latency
  // at all, so the check instead tracks an OUTSTANDING-transaction
  // COUNT: incremented whenever this channel's own request is accepted,
  // decremented whenever the matching response is accepted, checked
  // BEFORE the decrement takes effect in the same cycle. This makes NO
  // assumption about how many cycles elapse between accept and
  // response -- correct for arbitrary latency, including latency that
  // varies transaction to transaction -- but DOES rely on AXI4-Lite's
  // own in-order-response invariant (no transaction ID exists to
  // reorder against, so responses come back in request order); it also
  // cannot check that a given response carries the DATA belonging to
  // the request it is balancing against, only that the COUNT itself
  // never goes negative, which is the full extent of what a passive
  // monitor with no access to the DUT's internal state can verify.
  int unsigned outstanding_wr, outstanding_rd;

  always_ff @(posedge bus.clk_i or negedge bus.rst_ni) begin
    if (!bus.rst_ni) begin
      outstanding_wr <= '0;
    end else begin
      case ({
        bus.aw_valid && bus.aw_ready, bus.b_valid && bus.b_ready
      })
        2'b10:   outstanding_wr <= outstanding_wr + 1;
        2'b01:   outstanding_wr <= outstanding_wr - 1;
        default: outstanding_wr <= outstanding_wr;
      endcase
    end
  end

  always_ff @(posedge bus.clk_i or negedge bus.rst_ni) begin
    if (!bus.rst_ni) begin
      outstanding_rd <= '0;
    end else begin
      case ({
        bus.ar_valid && bus.ar_ready, bus.r_valid && bus.r_ready
      })
        2'b10:   outstanding_rd <= outstanding_rd + 1;
        2'b01:   outstanding_rd <= outstanding_rd - 1;
        default: outstanding_rd <= outstanding_rd;
      endcase
    end
  end

  property p_b_after_aw;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni) bus.b_valid |-> outstanding_wr > 0;
  endproperty
  a_b_after_aw :
  assert property (p_b_after_aw)
  else $error("axi4_lite_monitor: B response with no outstanding accepted write");

  property p_r_after_ar;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni) bus.r_valid |-> outstanding_rd > 0;
  endproperty
  a_r_after_ar :
  assert property (p_r_after_ar)
  else $error("axi4_lite_monitor: R response with no outstanding accepted read");

  // AXI4-Lite has only two legal RESP encodings in this design: OKAY
  // and SLVERR are the only ones any slave here ever produces. This
  // DUT (sram_bank.sv) hardwires `b_resp_o = 2'b00' (OKAY) always, so
  // the SLVERR arm of this check is dead for it specifically -- kept
  // anyway as a genuine protocol-level check, one that would start
  // firing the moment any slave connected to this monitor actually
  // drives SLVERR, but readers should not take its presence as
  // evidence that the error-response path is exercised by this demo.
  property p_b_resp_legal;
    @(posedge bus.clk_i) disable iff (!bus.rst_ni)
    bus.b_valid |-> (bus.b_resp == 2'b00 || bus.b_resp == 2'b10);
  endproperty
  a_b_resp_legal :
  assert property (p_b_resp_legal)
  else $error("axi4_lite_monitor: illegal B RESP encoding");

endmodule : axi4_lite_monitor
