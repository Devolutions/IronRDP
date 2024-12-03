namespace NowProto
{
    /// <summary>
    /// NowMessage definition.
    /// </summary>
    public interface INowMessage
    {
        /// <summary>
        /// NowProto message class.
        /// </summary>
        static abstract byte TypeMessageClass { get; }

        /// <summary>
        /// NowProto message kind (class-specific).
        /// </summary>
        static abstract byte TypeMessageKind { get; }

        // Workaround for C# not supporting accessing static abstract properties
        // from descendant interfaces (TypeMessageClass & TypeMessageKind only
        // useful in generic context)

        internal byte MessageClass { get; }
        internal byte MessageKind { get; }
    }
}
