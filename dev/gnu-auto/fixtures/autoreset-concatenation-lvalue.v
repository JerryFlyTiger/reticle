module p5 (input clk, input rst_n);
   reg a, b, c;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         {a, b} <= 2'b11;
         c <= 1'b1;
      end
   end
endmodule
