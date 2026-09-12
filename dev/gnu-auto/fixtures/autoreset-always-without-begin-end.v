module r9 (input clk, input rst_n);
   reg [7:0] a;
   reg       b;
   always @(posedge clk or negedge rst_n)
     if (!rst_n) begin
        /*AUTORESET*/
     end
     else begin
        a <= 8'd1;
        b <= 1'b1;
     end
endmodule
