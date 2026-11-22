using System; using System.Collections.Generic; using System.Diagnostics;
class Box { public double s = 1; }
class Outer { public Box inner = new Box(); }
class Shape { public virtual double area() { return 1; } }
class Sq : Shape { public override double area() { return 2; } }
static class Counter { public static double total = 0; }
static class P {
  const double N = 3000000; static double g_state = 1;
  static double b_local(){ double s=1,i=0; while(i<N){ s=(s*1664525+1013904223)%4294967296; i=i+1;} return s; }
  static double b_static(){ g_state=1; double i=0; while(i<N){ g_state=(g_state*1664525+1013904223)%4294967296; i=i+1;} return g_state; }
  static double b_class_static(){ Counter.total=1; double i=0; while(i<N){ Counter.total=(Counter.total*1664525+1013904223)%4294967296; i=i+1;} return Counter.total; }
  static double b_field(){ var o=new Box(); double i=0; while(i<N){ o.s=(o.s*1664525+1013904223)%4294967296; i=i+1;} return o.s; }
  static double b_two_hops(){ var o=new Outer(); double i=0; while(i<N){ o.inner.s=(o.inner.s*1664525+1013904223)%4294967296; i=i+1;} return o.inner.s; }
  static double b_literal(){ var o=new Dictionary<string,object>{{"s",1.0}}; double i=0; while(i<N){ o["s"]=((double)o["s"]*1664525+1013904223)%4294967296; i=i+1;} return (double)o["s"]; }
  static double step(double s){ return (s*1664525+1013904223)%4294967296; }
  static double b_call(){ double s=1,i=0; while(i<N){ s=step(s); i=i+1;} return s; }
  static double b_virtual(){ Shape a=new Sq(); double s=0,i=0; while(i<N){ s=s+a.area(); i=i+1;} return s; }
  static double b_array(){ var a=new List<double>(); double i=0; while(i<1000){a.Add(i); i=i+1;} double s=0,k=0; while(k<3000){ int j=0; while(j<1000){ s=s+a[j]; j=j+1;} k=k+1;} return s; }
  static double b_alloc(){ double s=0,i=0; while(i<N){ var b=new Box(); s=s+b.s; i=i+1;} return s; }
  static double b_string(){ string s=""; double i=0; while(i<200000){ s=s+"x"; i=i+1;} return s.Length; }
  static double fib(double n){ return n<2?n:fib(n-1)+fib(n-2); }
  static double b_fib(){ return fib(30); }
  static void timed(string label, Func<double> run){ var t=Stopwatch.StartNew(); var r=run(); t.Stop(); Console.WriteLine(label+" "+t.Elapsed.TotalMilliseconds.ToString("F1")+" ms ("+r+")"); }
  static void Main(){
    timed("local       ",b_local); timed("mod_static  ",b_static); timed("class_static",b_class_static);
    timed("field       ",b_field); timed("two_hops    ",b_two_hops); timed("obj_literal ",b_literal);
    timed("call        ",b_call); timed("virtual     ",b_virtual); timed("array       ",b_array);
    timed("alloc       ",b_alloc); timed("string      ",b_string); timed("fib30       ",b_fib);
  }
}
