module q1 (input clk);
   reg a, b;
   always @(posedge clk) begin
      /*AUTORESET*/
      a <= 1'b1;
      b <= 1'b1;
   end
endmodule
