module r10 (input clk, input rst_n);
   reg [7:0] a;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) a <= 8'h0;  /*AUTORESET*/
      else        a <= 8'd1;
   end
endmodule
