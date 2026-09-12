module p1 (input clk, input rst_n);
   reg a, b;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
         a <= 1'b0;
      end
      else begin
         a <= 1'b1;
         b <= 1'b1;
      end
   end
endmodule
