module r11 #(parameter WIDTH = 8) (input clk, input rst_n);
   reg [WIDTH-1:0]   pw;
   reg [WIDTH/2-1:0] hw;
   reg [3:2]         nz;
   always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
         /*AUTORESET*/
      end
      else begin
         pw <= 1;
         hw <= 1;
         nz <= 1;
      end
   end
endmodule
