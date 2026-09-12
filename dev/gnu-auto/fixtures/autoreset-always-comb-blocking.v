module p6 (input clk, input rst_n);
   reg a, b;
   always_comb begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         a = 1'b1;
         b = 1'b1;
      end
   end
endmodule
