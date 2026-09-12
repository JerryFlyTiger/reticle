module p7 (input clk, input rst_n);
   reg b;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         top.inner.sig <= 1'b1;
         b <= 1'b1;
      end
   end
endmodule
