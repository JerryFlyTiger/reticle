module r4 (input clk, input rst_n);
   reg        zebra;
   reg [15:0] apple;
   reg signed [3:0] mango;
   reg [7:0]  mem [0:3];
   logic [2:0] lg;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         zebra <= 1'b1;
         apple <= 16'd5;
         mango <= -1;
         mem[0] <= 8'd1;
         lg <= 3'd2;
      end
   end
endmodule
