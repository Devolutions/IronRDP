public class ByteList {

    private byte[] inner;
    private int occupied = 0;
    private int capacity;

    public ByteList(int capacity) {
        this.capacity = capacity;
        this.inner = new byte[capacity];
    }

    public ByteList(int capacity, byte[] bytes) {
        this.capacity = capacity;
        this.inner = new byte[capacity];
        this.AddRange(bytes);
    }

    public byte[] Peek() {
        return java.util.Arrays.copyOf(inner, occupied);
    }

    public void AddRange(byte[] bytes) {
        while (bytes.length + occupied > capacity) {
            this.Grow();
        }
        System.arraycopy(bytes, 0, this.inner, occupied, bytes.length);
        occupied += bytes.length;
    }

    public ByteList SplitTo(int at) {
        if (at > occupied) {
            throw new IllegalArgumentException("Not enough bytes to split");
        }

        byte[] res = new byte[at];
        System.arraycopy(this.inner, 0, res, 0, at);
        System.arraycopy(this.inner, at, this.inner, 0, occupied - at);
        occupied -= at;

        ByteList resultByteList = new ByteList(capacity, res);
        return resultByteList;
    }

    private void Grow() {
        if (capacity == 0) {
            capacity = 1;
        } else {
            capacity *= 2;
        }

        byte[] newInner = new byte[capacity];
        System.arraycopy(inner, 0, newInner, 0, occupied);
        inner = newInner;
    }
}
