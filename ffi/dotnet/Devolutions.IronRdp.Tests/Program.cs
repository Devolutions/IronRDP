using System.Runtime.InteropServices;

namespace Devolutions.IronRdp.Tests;

internal static class Program
{
    private static int Main()
    {
        Console.WriteLine($"Runtime: {RuntimeInformation.FrameworkDescription}, {RuntimeInformation.ProcessArchitecture}");

        int passed = 0, failed = 0;
        foreach (var (name, run) in SmokeTests.Cases())
        {
            try
            {
                run();
                passed++;
                Console.WriteLine($"PASS {name}");
            }
            catch (Exception error)
            {
                failed++;
                Console.Error.WriteLine($"FAIL {name}\n{error}");
            }
        }

        Console.WriteLine($"RESULT: {passed}/{passed + failed} passed");
        return failed == 0 ? 0 : 1;
    }
}
