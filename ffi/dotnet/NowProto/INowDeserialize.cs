namespace NowProto
{
    /// <summary>
    /// Deserializable NowProto message.
    /// </summary>
    public interface INowDeserialize<out T> : INowMessage
    {
        static abstract T Deserialize(ushort flags, NowReadCursor cursor);
    }
}
