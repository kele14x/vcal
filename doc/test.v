`timescale 1ps / 1ps
//
`default_nettype none

module test ();

  reg [3:0] a;
  reg [3:0] b;
  reg [3:0] c;
  reg [3:0] p;

  initial begin
    $display("p = %b", +4'b01zx);
  end

endmodule

`default_nettype wire
