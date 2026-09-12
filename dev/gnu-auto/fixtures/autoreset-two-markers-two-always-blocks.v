module r15 (input clk, input rst_n);
   reg a, b;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else a <= 1'b1;
   end
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else b <= 1'b1;
   end
endmodule
