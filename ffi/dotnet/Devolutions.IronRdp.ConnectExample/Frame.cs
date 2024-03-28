using Devolutions.IronRdp;

public class Framed<S> where S : Stream
{
    private S stream;
    private List<byte> buffer;

    public Framed(S stream)
    {
        this.stream = stream;
        this.buffer = new List<byte>();
    }

    public (S, List<byte>) GetInner()
    {
        return (this.stream, this.buffer);
    }

    public byte[] Peek()
    {
        return this.buffer.ToArray();
    }

    public async Task ReadExact(nuint size)
    {
        var buffer = new byte[size];
        Memory<byte> memory = buffer;
        await this.stream.ReadExactlyAsync(memory);
        this.buffer.AddRange(buffer);
    }

    async Task<int> Read() {
        var buffer = new byte[1024];
        Memory<byte> memory = buffer;
        var size = await this.stream.ReadAsync(memory);
        this.buffer.AddRange(buffer.Take(size));
        return size;
    }

    public async Task Write(byte[] data)
    {
        ReadOnlyMemory<byte> memory = data;
        await this.stream.WriteAsync(memory);
    }


    public async Task<byte[]> ReadByHint(PduHint pduHint) {
        while(true) {

            var size = pduHint.FindSize(this.buffer.ToArray());

            if (size.IsSome()) {
                await this.ReadExact(size.Get());
                return this.buffer.ToArray();
            }else {
                var len = await this.Read();
                if (len == 0) {
                    throw new Exception("EOF");
                }
            }

        }
    }
}
