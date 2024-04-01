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

    // read from 0 to size bytes from the front of the buffer, and remove them from the buffer,keep the rest
    public async Task<byte[]> ReadExact(nuint size)
    {
        while (true)
        {
            if (buffer.Count >= (int)size)
            {
                var res = this.buffer.Take((int)size).ToArray();
                this.buffer = this.buffer.Skip((int)size).ToList();
                return res;
            }

            var len = await this.Read();
            if (len == 0)
            {
                throw new Exception("EOF");
            }
        }
    }

    async Task<int> Read()
    {
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


    public async Task<byte[]> ReadByHint(PduHint pduHint)
    {
        while (true)
        {
            var size = pduHint.FindSize(this.buffer.ToArray());
            if (size.IsSome())
            {
                return await this.ReadExact(size.Get());
            }
            else
            {
                var len = await this.Read();
                if (len == 0)
                {
                    throw new Exception("EOF");
                }
            }

        }
    }
}
