// soc_verif_pkg.sv -- the read/write checker class shared by the demo
// testbenches.
//
// Icarus Verilog 13.0 crashes compiling a class with an unpacked-array
// member (`Assertion failed ... draw_class.c:41`) and rejects a queue
// inside a class outright ("Queues inside classes are not yet
// supported"), both dump-verified with minimal repros before this file
// was written. `rw_checker` therefore keeps scalar members only: one
// expected value and two running counters, checked one transaction at
// a time rather than recorded into a collection.

package soc_verif_pkg;

  // Told the expected value before a transaction, shown the observed
  // value after it completes, and keeps a running hit/miss count so the
  // testbench can print one summary line at the end instead of a
  // $display per transaction.
  class rw_checker;
    protected int unsigned hits_q;
    protected int unsigned misses_q;
    protected logic [31:0] expected_q;

    function new();
      hits_q     = 0;
      misses_q   = 0;
      expected_q = '0;
    endfunction

    function automatic void set_expected(logic [31:0] value);
      expected_q = value;
    endfunction

    // Compares `value` against whatever set_expected() was last told,
    // bumps the matching counter, and returns the comparison result so
    // the caller can react immediately as well as at the end.
    function automatic bit check(logic [31:0] value);
      bit ok;
      ok = (value === expected_q);
      if (ok) begin
        hits_q++;
      end else begin
        misses_q++;
      end
      return ok;
    endfunction

    function automatic int unsigned hit_count();
      return hits_q;
    endfunction

    function automatic int unsigned miss_count();
      return misses_q;
    endfunction

    function automatic string summary();
      return $sformatf("hits=%0d misses=%0d", hits_q, misses_q);
    endfunction
  endclass : rw_checker

endpackage : soc_verif_pkg
