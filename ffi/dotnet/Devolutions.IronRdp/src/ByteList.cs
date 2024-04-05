
public class ByteList {

    private inner: byte[]
    private occupied: int

    public ByteList(int capacity) {
        this.capacity = capacity
        this.inner = new byte[capacity]
    }

    public ByteList(int capacity, byte[] bytes) {
        this.capacity = capacity
        this.inner = new byte[capacity]
        this.AddRange(bytes)
    }

    public byte[] Peek() {
        return this.inner
    }

    public void AddRange(byte[] bytes) {
        while (bytes.Length > this.capacity) {
            this.Grow()
        }
        Array.Copy(bytes, 0, this.inner, 0, bytes.Length)
    }

    /// <summary>
    /// Split the ByteList into two parts. The first part will returned and removed from the ByteList. The second part will remain in the ByteList.
    /// The Returned Part will be contain [0, at) bytes from the ByteList. and the ByteList will contain [at, occupied) bytes.
    /// </summary>
    /// <param name="at">The size of the byte array to split.</param>
    /// <returns>A new byte array containing the split bytes.</returns>
    public ByteList SplitTo(int at) {
        if (at > this.occupied) {
            throw new Exception("Not enough bytes to split")
        }

        var res = new byte[at]
        Array.Copy(this.inner, 0, res, 0, at)
        Array.Copy(this.inner, at, this.inner, 0, this.occupied - at)
        this.occupied -= at
        return ByteList(at, res)
    }

    private void Grow() {
        if (this.capacity == 0) {
            this.capacity = 1
        } else {
            this.capacity *= 2
        }

        var newInner = new byte[this.capacity]
        Array.Copy(this.inner, 0, newInner, 0, this.occupied)
    }
}