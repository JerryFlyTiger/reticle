module r16 (input clk, input rst_n);
   reg a, b;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) b <= 1'b0;
      else        a <= 1'b1;
      /*AUTORESET*/
   end
endmodule
