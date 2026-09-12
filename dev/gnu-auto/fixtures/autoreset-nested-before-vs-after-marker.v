module p3 (input clk, input rst_n, input x);
   reg a, b, c;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         if (x) a <= 1'b0;
         /*AUTORESET*/
         if (x) c <= 1'b0;
      end
      else begin
         a <= 1'b1; b <= 1'b1; c <= 1'b1;
      end
   end
endmodule
