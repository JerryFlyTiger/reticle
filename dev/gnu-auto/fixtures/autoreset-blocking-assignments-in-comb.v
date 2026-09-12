module r3 (input clk, input rst_n, input [7:0] d, output reg [7:0] q);
   reg [3:0] cnt;
   always @* begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         q = d;
         cnt = 4'd3;
      end
   end
endmodule
