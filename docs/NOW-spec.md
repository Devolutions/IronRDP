
# NOW-PROTO

<style>
    .byte-layout {
        width: 100%;
        table-layout: fixed;
    }
    .byte-layout th {
        colspan: 1;
        text-align: center;
        vertical-align: bottom;
    }
    .byte-layout td {
        text-align: center;
    }
</style>


[[_TOC_]]


# Messages

## Transport

The NOW virtual channel protocol use an RDP dynamic virtual channel ("Devolutions::Now::Agent") as a transport type.

## Message Syntax

The following sections specify the NOW protocol message syntax. Unless otherwise specified, all fields defined in this document use the little-endian format.

### Common Structures

#### NOW_INTEGER

Signed and unsigned integer encoding structures of various sizes.

##### NOW_VARU16

The NOW_VARU16 structure is used to encode unsigned integer values in the range [0, 0x7FFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="1">c</td>
            <td colspan="7">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="16"></td>
        </tr>
    </tbody>
</table>

**c (1 bit)**: A 1-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |

**val1 (7 bits)**: A 7-bit integer containing the 7 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: A 8-bit integer containing the least significant bits of the integer value represented by this structure.

##### NOW_VARI16

The NOW_VARI16 structure is used to encode signed integer values in the range [-0x3FFF, 0x3FFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="1">c</td>
            <td colspan="1">s</td>
            <td colspan="6">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="16"></td>
        </tr>
    </tbody>
</table>

**c (1 bit)**: A 1-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |

**s (1 bit)**: A 1-bit integer containing the encoded sign representation of the integer value.

| Value | Meaning |
|-------|---------|
| 0 | Positive value |
| 1 | Negative value |

**val1 (6 bits)**: A 6-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: A 8-bit integer containing the least significant bits of the integer value represented by this structure.

##### NOW_VARU32

The NOW_VARU32 structure is used to encode signed integer values in the range [0, 0x3FFFFFFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="2">c</td>
            <td colspan="6">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="8">val3 (optional)</td>
            <td colspan="8">val4 (optional)</td>
        </tr>
    </tbody>
</table>

**c (2 bits)**: A 2-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |
| 2 | The val1, val2, val3 fields are present (3 bytes) |
| 3 | The val1, val2, val3, val4 fields are present (4 bytes) |

**val1 (6 bits)**: A 6-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: An 8-bit integer containing the second most significant bits of the integer value represented by this structure.

**val3 (1 byte)**: An 8-bit integer containing the third most significant bits of the integer value represented by this structure.

**val4 (1 byte)**: An 8-bit integer containing the least significant bits of the integer value represented by this structure.

##### NOW_VARI32

The NOW_VARI32 structure is used to encode signed integer values in the range [-0x1FFFFFFF, 0x1FFFFFFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="2">c</td>
            <td colspan="1">s</td>
            <td colspan="5">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="8">val3 (optional)</td>
            <td colspan="8">val4 (optional)</td>
        </tr>
    </tbody>
</table>

**c (2 bits)**: A 2-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |
| 2 | The val1, val2, val3 fields are present (3 bytes) |
| 3 | The val1, val2, val3, val4 fields are present (4 bytes) |

**s (1 bit)**: A 1-bit integer containing the encoded sign representation of the integer value.

| Value | Meaning |
|-------|---------|
| 0 | Positive value |
| 1 | Negative value |

**val1 (5 bits)**: A 5-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: An 8-bit integer containing the second most significant bits of the integer value represented by this structure.

**val3 (1 byte)**: An 8-bit integer containing the third most significant bits of the integer value represented by this structure.

**val4 (1 byte)**: An 8-bit integer containing the least significant bits of the integer value represented by this structure.

##### NOW_VARU64

The NOW_VARU64 structure is used to encode signed integer values in the range [0, 0x3FFFFFFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="3">c</td>
            <td colspan="5">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="8">val3 (optional)</td>
            <td colspan="8">val4 (optional)</td>
        </tr>
        <tr>
            <td colspan="8">val5 (optional)</td>
            <td colspan="8">val6 (optional)</td>
            <td colspan="8">val7 (optional)</td>
            <td colspan="8">val8 (optional)</td>
        </tr>
    </tbody>
</table>

**c (3 bits)**: A 3-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |
| 2 | The val1, val2, val3 fields are present (3 bytes) |
| 3 | The val1, val2, val3, val4 fields are present (4 bytes) |
| 4 | The val1, val2, val3, val4, val5 fields are present (5 bytes) |
| 5 | The val1, val2, val3, val4, val5, val6 fields are present (6 bytes) |
| 6 | The val1, val2, val3, val4, val5, val6, val7 fields are present (7 bytes) |
| 7 | The val1, val2, val3, val4, val5, val6, val7, val8 fields are present (8 bytes) |

**val1 (5 bits)**: A 5-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: An 8-bit integer containing the second most significant bits of the integer value represented by this structure.

**val3 (1 byte)**: An 8-bit integer containing the third most significant bits of the integer value represented by this structure.

**val4 (1 byte)**: An 8-bit integer containing the fourth significant bits of the integer value represented by this structure.

**val5 (1 byte)**: An 8-bit integer containing the fifth significant bits of the integer value represented by this structure.

**val6 (1 byte)**: An 8-bit integer containing the sixth significant bits of the integer value represented by this structure.

**val7 (1 byte)**: An 8-bit integer containing the seventh significant bits of the integer value represented by this structure.

**val8 (1 byte)**: An 8-bit integer containing the least significant bits of the integer value represented by this structure.

##### NOW_VARI64

The NOW_VARI64 structure is used to encode signed integer values in the range [-0x1FFFFFFF, 0x1FFFFFFF].

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="3">c</td>
            <td colspan="1">s</td>
            <td colspan="4">val1</td>
            <td colspan="8">val2 (optional)</td>
            <td colspan="8">val3 (optional)</td>
            <td colspan="8">val4 (optional)</td>
        </tr>
    </tbody>
</table>

**c (3 bits)**: A 3-bit integer containing an encoded representation of the number of bytes in this structure.

| Value | Meaning |
|-------|---------|
| 0 | The val1 field is present (1 byte) |
| 1 | The val1, val2 fields are present (2 bytes) |
| 2 | The val1, val2, val3 fields are present (3 bytes) |
| 3 | The val1, val2, val3, val4 fields are present (4 bytes) |
| 4 | The val1, val2, val3, val4, val5 fields are present (5 bytes) |
| 5 | The val1, val2, val3, val4, val5, val6 fields are present (6 bytes) |
| 6 | The val1, val2, val3, val4, val5, val6, val7 fields are present (7 bytes) |
| 7 | The val1, val2, val3, val4, val5, val6, val7, val8 fields are present (8 bytes) |

**s (1 bit)**: A 1-bit integer containing the encoded sign representation of the integer value.

| Value | Meaning |
|-------|---------|
| 0 | Positive value |
| 1 | Negative value |

**val1 (4 bits)**: A 4-bit integer containing the 6 most significant bits of the integer value represented by this structure.

**val2 (1 byte)**: An 8-bit integer containing the second most significant bits of the integer value represented by this structure.

**val3 (1 byte)**: An 8-bit integer containing the third most significant bits of the integer value represented by this structure.

**val4 (1 byte)**: An 8-bit integer containing the fourth significant bits of the integer value represented by this structure.

**val5 (1 byte)**: An 8-bit integer containing the fifth significant bits of the integer value represented by this structure.

**val6 (1 byte)**: An 8-bit integer containing the sixth significant bits of the integer value represented by this structure.

**val7 (1 byte)**: An 8-bit integer containing the seventh significant bits of the integer value represented by this structure.

**val8 (1 byte)**: An 8-bit integer containing the least significant bits of the integer value represented by this structure.

#### NOW_STRING

##### NOW_VARSTR

The NOW_VARSTR structure is used to represent variable-length strings that could be large, while remaining compact in size for small strings.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">len (variable)</td>
        </tr>
        <tr>
            <td colspan="32">str (variable)</td>
        </tr>
    </tbody>
</table>

**len (variable)**: A NOW_VARU32 structure containing the string length, excluding the null terminator.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_LRGSTR

The NOW_LRGSTR structure is used to represent large variable-length strings.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">len</td>
        </tr>
        <tr>
            <td colspan="32">str (variable)</td>
        </tr>
    </tbody>
</table>

**len (4 bytes)**: A 32-bit unsigned integer containing the string length, excluding the null terminator.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_STRING16

The NOW_STRING16 structure is used to represent variable-length strings of up to 15 characters that can easily fit within a fixed-size buffer of 16 bytes.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="8">len</td>
            <td colspan="24">str (variable)</td>
        </tr>
        <tr>
            <td colspan="32">...</td>
        </tr>
    </tbody>
</table>

**len (1 byte)**: An unsigned 8-bit number containing the string length, excluding the null terminator. The maximum value is 15.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_STRING32

The NOW_STRING32 structure is used to represent variable-length strings of up to 15 characters that can easily fit within a fixed-size buffer of 32 bytes.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="8">len</td>
            <td colspan="24">str (variable)</td>
        </tr>
        <tr>
            <td colspan="32">...</td>
        </tr>
    </tbody>
</table>

**len (1 byte)**: An unsigned 8-bit number containing the string length, excluding the null terminator. The maximum value is 31.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_STRING64

The NOW_STRING64 structure is used to represent variable-length strings of up to 63 characters that can easily fit within a fixed-size buffer of 64 bytes.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="8">len</td>
            <td colspan="24">str (variable)</td>
        </tr>
        <tr>
            <td colspan="32">...</td>
        </tr>
    </tbody>
</table>

**len (1 byte)**: An unsigned 8-bit number containing the string length, excluding the null terminator. The maximum value is 63.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_STRING128

The NOW_STRING128 structure is used to represent variable-length strings of up to 127 characters that can easily fit within a fixed-size buffer of 128 bytes.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="8">len</td>
            <td colspan="24">str (variable)</td>
        </tr>
        <tr>
            <td colspan="32">...</td>
        </tr>
    </tbody>
</table>

**len (1 byte)**: An unsigned 8-bit number containing the string length, excluding the null terminator. The maximum value is 127.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

##### NOW_STRING256

The NOW_STRING256 structure is used to represent variable-length strings of up to 255 characters that can easily fit within a fixed-size buffer of 256 bytes.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="8">len</td>
            <td colspan="24">str (variable)</td>
        </tr>
        <tr>
            <td colspan="32">...</td>
        </tr>
    </tbody>
</table>

**len (1 byte)**: An unsigned 8-bit number containing the string length, excluding the null terminator. The maximum value is 255.

**str (variable)**: The UTF-8 encoded string excluding the null terminator.

#### NOW_BUFFER

##### NOW_VARBUF

The NOW_VARBUF structure is used to represent variable-length buffers.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">size (variable)</td>
        </tr>
        <tr>
            <td colspan="32">data (variable)</td>
        </tr>
    </tbody>
</table>

**size (variable)**: A NOW_VARU32 structure containing the buffer size.

**data (variable)**: The buffer data, whose size is given by the size field.

##### NOW_LRGBUF

The NOW_LRGBUF structure is used to represent variable-length buffers.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">size</td>
        </tr>
        <tr>
            <td colspan="32">data (variable)</td>
        </tr>
    </tbody>
</table>

**size (4 bytes)**: A 32-bit unsigned integer containing the buffer size.

**data (variable)**: The buffer data, whose size is given by the size field.

#### NOW_HEADER

The NOW_HEADER structure is the header common to all NOW protocol messages.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class.

| Flag                            | Meaning              |
|---------------------------------|----------------------|
| NOW_SYSTEM_MSG_CLASS_ID<br>0x11 | System message class |
| NOW_SESSION_MSG_CLASS_ID<br>0x12 | Session message class |
| NOW_EXEC_MSG_CLASS_ID<br>0x13 | Exec message class |

**msgType (1 byte)**: The message type, specific to the message class.

**msgFlags (2 bytes)**: The message flags, specific to the message type and class.

#### NOW_STATUS
A status code, with a structure similar to HRESULT.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="2">S</td>
            <td colspan="1">Y</td>
            <td colspan="1">Z</td>
            <td colspan="4">class</td>
            <td colspan="8">type</td>
            <td colspan="16">code</td>
        </tr>
    </tbody>
</table>

**S (2 bits)**: Severity level.

| Value | Meaning |
|-------|---------|
| NOW_SEVERITY_INFO<br>0 | Informative status |
| NOW_SEVERITY_WARN<br>1 | Warning status |
| NOW_SEVERITY_ERROR<br>2 | Error status (recoverable) |
| NOW_SEVERITY_FATAL<br>3 | Error status (non-recoverable) |

**Y (1 bit)**: Reserved. MUST be set to zero.

**Z (1 bit)**: Reserved. MUST be set to zero.

**class (4 bits)**: Reserved. MUST be set to zero.

**type (8 bits)**: The status type.

**code (16 bits)**: The status code.

| Value | Meaning |
|-------|---------|
| NOW_CODE_SUCCESS<br>0x0000 | Success |
| NOW_CODE_FAILURE<br>0xFFFF | Failure |
| NOW_CODE_FILE_NOT_FOUND<br>0x0002 | File not found. |
| NOW_CODE_ACCESS_DENIED<br>0x0005 | File not found. |
| NOW_CODE_BAD_FORMAT<br>0x000B | The program has an incorrect or bad format. |

### System Messages

#### NOW_SYSTEM_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SYSTEM_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value                           | Meaning              |
|---------------------------------|----------------------|
| NOW_SYSTEM_INFO_REQ_ID<br>0x01 | NOW_SYSTEM_INFO_REQ_MSG |
| NOW_SYSTEM_INFO_RSP_ID<br>0x02 | NOW_SYSTEM_INFO_RSP_MSG |
| NOW_SYSTEM_SHUTDOWN_ID<br>0x03 | NOW_SYSTEM_SHUTDOWN_MSG |

<!-- TODO: Define NOW_SYSTEM_INFO_REQ_MSG, NOW_SYSTEM_INFO_RSP_MSG   -->

#### NOW_SYSTEM_SHUTDOWN_MSG

The NOW_SYSTEM_SHUTDOWN_MSG structure is used to request a system shutdown.NOW_SESSION_LOGOFF_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">timeout</td>
        </tr>
        <tr>
            <td colspan="32">message</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SYSTEM_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SYSTEM_SHUTDOWN_MSG_ID)

**msgFlags (2 bytes)**: The message flags.

| Flag | Meaning |
|------|---------|
| NOW_SHUTDOWN_FLAG_FORCE<br>0x0001 | Force shutdown |
| NOW_SHUTDOWN_FLAG_REBOOT<br>0x0002 | Reboot after shutdown |

**timeout (4 bytes)**: This system shutdown timeout, in seconds.

**message (variable)**: A NOW_STRING structure containing an optional shutdown message.

### Session Messages

#### NOW_SESSION_MSG

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value                           | Meaning              |
|---------------------------------|----------------------|
| NOW_SESSION_LOCK_MSG_ID<br>0x01 | NOW_SESSION_LOCK_MSG |
| NOW_SESSION_LOGOFF_MSG_ID<br>0x02 | NOW_SESSION_LOGOFF_MSG |
| NOW_SESSION_MESSAGE_BOX_MSG_REQ_ID<br>0x03 | NOW_SESSION_MESSAGE_BOX_MSG |
| NOW_SESSION_MESSAGE_BOX_RSP_MSG_ID<br>0x04 | NOW_SESSION_MESSAGE_RSP_MSG |

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_LOCK_MSG

The NOW_SESSION_LOCK_MSG is used to request locking the user session.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_LOCK_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_LOGOFF_MSG

The NOW_SESSION_LOGOFF_MSG is used to request a user session logoff.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_LOGOFF_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

#### NOW_SESSION_MSGBOX_REQ_MSG

The NOW_SESSION_MSGBOX_REQ_MSG is used to show a message box in the user session, similar to what the [WTSSendMessage function](https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtssendmessagew) does.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">requestId</td>
        </tr>
        <tr>
            <td colspan="32">style</td>
        </tr>
        <tr>
            <td colspan="32">timeout</td>
        </tr>
        <tr>
            <td colspan="32">title (variable)</td>
        </tr>
        <tr>
            <td colspan="32">text (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_MESSAGE_BOX_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                | Meaning                                 |
|-------------------------------------|-----------------------------------------|
| NOW_MSGBOX_FLAG_TITLE<br>0x00000001 | The title field contains a non-default value |
| NOW_MSGBOX_FLAG_STYLE<br>0x00000002 | The style field contains a non-default value |
| NOW_MSGBOX_FLAG_TIMEOUT<br>0x00000004 | The timeout field contains a non-default value |
| NOW_MSGBOX_FLAG_RESPONSE<br>0x00000008 | A response message is expected (don't fire and forget) |

**requestId (4 bytes)**: the message request id, sent back in the response.

**style (4 bytes)**: The message box style, ignored if NOW_MSGBOX_FLAG_STYLE is not set. MBOK is the default, refer to the [MessageBox function](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-messagebox) for all possible styles. This field may be ignored on platforms other than Windows.

**timeout (4 bytes)**: The timeout, in seconds, that the message box dialog should wait for the user response. This value is NOW_MSGBOX_FLAG_TIMEOUT is not set.

**title (variable)**: The message box title, ignored if NOW_MSGBOX_FLAG_TITLE is not set.

**text (variable)**: The message box text.

#### NOW_SESSION_MSGBOX_RSP_MSG

The NOW_SESSION_MSGBOX_RSP_MSG is a message sent in response to NOW_SESSION_MSGBOX_REQ_MSG if the NOW_MSGBOX_FLAG_RESPONSE has been set, and contains the result from the message box dialog.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">requestId</td>
        </tr>
        <tr>
            <td colspan="32">response</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_SESSION_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_SESSION_MESSAGE_RSP_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**requestId (4 bytes)**: The corresponding message box request id.

**response (4 bytes)**: The message box response.

| Value        | Meaning |
|--------------|---------|
| IDABORT<br>3 | Abort   |
| IDCANCEL<br>2 | Cancel   |
| IDCONTINUE<br>11 | Continue   |
| IDIGNORE<br>5 | Ignore   |
| IDNO<br>7 | No   |
| IDOK<br>1 | OK   |
| IDRETRY<br>4 | Retry   |
| IDTRYAGAIN<br>10 | Try Again   |
| IDYES<br>6 | Yes   |
| IDTIMEOUT<br>32000 | Timeout   |

### Execution Messages

#### NOW_EXEC_MSG

The NOW_EXEC_MSG message is used to execute remote commands or scripts.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type.

| Value | Meaning |
|-------|---------|
| NOW_EXEC_CAPSET_MSG_ID<br>0x00 | NOW_EXEC_CAPSET_MSG |
| NOW_EXEC_ABORT_MSG_ID<br>0x01 | NOW_EXEC_ABORT_MSG |
| NOW_EXEC_CANCEL_REQ_MSG_ID<br>0x02 | NOW_EXEC_CANCEL_REQ_MSG |
| NOW_EXEC_CANCEL_RSP_MSG_ID<br>0x03 | NOW_EXEC_CANCEL_RSP_MSG |
| NOW_EXEC_RESULT_MSG_ID<br>0x04 | NOW_EXEC_RESULT_MSG |
| NOW_EXEC_DATA_MSG_ID<br>0x05 | NOW_EXEC_DATA_MSG |
| NOW_EXEC_RUN_MSG_ID<br>0x10 | NOW_EXEC_RUN_MSG |
| NOW_EXEC_CMD_MSG_ID<br>0x11 | NOW_EXEC_CMD_MSG |
| NOW_EXEC_PROCESS_MSG_ID<br>0x12 | NOW_EXEC_PROCESS_MSG |
| NOW_EXEC_SHELL_MSG_ID<br>0x13 | NOW_EXEC_SHELL_MSG |
| NOW_EXEC_BATCH_MSG_ID<br>0x14 | NOW_EXEC_BATCH_MSG |
| NOW_EXEC_WINPS_MSG_ID<br>0x15 | NOW_EXEC_WINPS_MSG |
| NOW_EXEC_PWSH_MSG_ID<br>0x16 | NOW_EXEC_PWSH_MSG |

**msgFlags (2 bytes)**: The message flags.

#### NOW_EXEC_CAPSET_MSG

The NOW_EXEC_CAPSET_MSG message is sent to advertise capabilities.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_CAPSET_MSG_ID).

**msgFlags (2 bytes)**: A 16-bit unsigned integer containing the supported execution styles.

| Flag | Meaning |
|-------|---------|
| NOW_EXEC_STYLE_RUN<br>0x0001 | Generic "Run" execution style. |
| NOW_EXEC_STYLE_CMD<br>0x0002 | Generic command execution style. |
| NOW_EXEC_STYLE_PROCESS<br>0x0004 | CreateProcess() execution style. |
| NOW_EXEC_STYLE_SHELL<br>0x0008 | System shell (.sh) execution style. |
| NOW_EXEC_STYLE_BATCH<br>0x0010 | Windows batch file (.bat) execution style. |
| NOW_EXEC_STYLE_WINPS<br>0x0020 | Windows PowerShell (.ps1) execution style. |
| NOW_EXEC_STYLE_PWSH<br>0x0040 | PowerShell 7 (.ps1) execution style. |
| NOW_EXEC_STYLE_APPLESCRIPT<br>0x0080 | Applescript (.scpt) execution style. |

<!-- TODO: add AppleScript command -->

#### NOW_EXEC_ABORT_MSG

The NOW_EXEC_ABORT_MSG message is used to abort a remote execution immediately due to an unrecoverable error. This message can be sent at any time without an explicit response message. The session is considered aborted as soon as this message is sent.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">status</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_ABORT_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**status (4 bytes)**: A NOW_STATUS error code.

#### NOW_EXEC_CANCEL_REQ_MSG

The NOW_EXEC_CANCEL_REQ_MSG message is used to cancel a remote execution session.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_CANCEL_REQ_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

#### NOW_EXEC_CANCEL_RSP_MSG

The NOW_EXEC_CANCEL_RSP_MSG message is used to respond to a remote execution cancel request.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">status</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_CANCEL_RSP_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**status (4 bytes)**: A NOW_STATUS error code.

#### NOW_EXEC_RESULT_MSG

The NOW_EXEC_RESULT_MSG message is used to return the result of an execution request.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">status</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_RESULT_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**status (4 bytes)**: A NOW_STATUS error code.

#### NOW_EXEC_DATA_MSG

The NOW_EXEC_DATA_MSG message is used to send input/output data as part of a remote execution.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">data (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_DATA_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

| Flag                                   | Meaning                         |
|----------------------------------------|---------------------------------|
| NOW_EXEC_FLAG_DATA_FIRST<br>0x00000001 | This is the first data message. |
| NOW_EXEC_FLAG_DATA_LAST<br>0x00000002 | This is the last data message, the command completed execution. |
| NOW_EXEC_FLAG_DATA_STDIN<br>0x00000004 | The data is from the standard input. |
| NOW_EXEC_FLAG_DATA_STDOUT<br>0x00000008 | The data is from the standard output. |
| NOW_EXEC_FLAG_DATA_STDERR<br>0x00000010 | The data is from the standard error. |

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**data (variable)**: The input/output data represented as `NOW_VARBUF`

#### NOW_EXEC_RUN_MSG

The NOW_EXEC_RUN_MSG message is used to send a run request. This request type maps to starting a program by using the “Run” menu on operating systems (the Start Menu on Windows, the Dock on macOS etc.). The execution of programs started with NOW_EXEC_RUN_MSG is not followed and does not send back the output.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_RUN_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

#### NOW_EXEC_CMD_MSG
<!-- TODO: Define CMD message -->

#### NOW_EXEC_PROCESS_MSG

The NOW_EXEC_PROCESS_MSG message is used to send a Windows [CreateProcess()](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw) request.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">filename (variable)</td>
        </tr>
        <tr>
            <td colspan="32">parameters (variable)</td>
        </tr>
        <tr>
            <td colspan="32">directory (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_PROCESS_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**filename (variable)**: A NOW_VARSTR structure containing the file name. Corresponds to the lpApplicationName parameter.

**parameters (variable)**: A NOW_VARSTR structure containing the command parameters. Corresponds to the lpCommandLine parameter.

**directory (variable)**: A NOW_VARSTR structure containing the command working directory. Corresponds to the lpCurrentDirectory parameter.

#### NOW_EXEC_SHELL_MSG

The NOW_EXEC_SHELL_MSG message is used to execute a remote shell command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">shell (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_SHELL_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

**shell (variable)**: A NOW_VARSTR structure containing the shell to use for execution. If no shell is specified, the default system shell (/bin/sh) will be used.

#### NOW_EXEC_BATCH_MSG

The NOW_EXEC_BATCH_MSG message is used to execute a remote batch command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_BATCH_MSG_ID).

**msgFlags (2 bytes)**: The message flags.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

#### NOW_EXEC_WINPS_MSG

The NOW_EXEC_WINPS_MSG message is used to execute a remote Windows PowerShell (powershell.exe) command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">executionPolicy (variable)</td>
        </tr>
        <tr>
            <td colspan="32">configurationName (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_WINPS_MSG_ID).

**msgFlags (2 bytes)**: The message flags, specifying the PowerShell command-line arguments.

| Flag                                   | Meaning                   |
|----------------------------------------|---------------------------|
| NOW_EXEC_FLAG_PS_NO_LOGO<br>0x00000001 | PowerShell -NoLogo option |
| NOW_EXEC_FLAG_PS_NO_EXIT<br>0x00000002 | PowerShell -NoExit option |
| NOW_EXEC_FLAG_PS_STA<br>0x00000004 | PowerShell -Sta option |
| NOW_EXEC_FLAG_PS_MTA<br>0x00000008 | PowerShell -Mta option |
| NOW_EXEC_FLAG_PS_NO_PROFILE<br>0x00000010 | PowerShell -NoProfile option |
| NOW_EXEC_FLAG_PS_NON_INTERACTIVE<br>0x00000020 | PowerShell -NonInteractive option |
| NOW_EXEC_FLAG_PS_EXECUTION_POLICY<br>0x00000040 | The PowerShell -ExecutionPolicy parameter is specified with value in executionPolicy field |
| NOW_EXEC_FLAG_PS_CONFIGURATION_NAME<br>0x00000080 | The PowerShell -ConfigurationName parameter is specified with value in configurationName field |

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**executionPolicy (variable)**: A NOW_VARSTR structure containing the execution policy (-ExecutionPolicy) parameter value. This value is ignored if the NOW_EXEC_FLAG_PS_EXECUTION_POLICY flag is not set.

**configurationName (variable)**: A NOW_VARSTR structure containing the configuration name (-ConfigurationName) parameter value. This value is ignored if the NOW_EXEC_FLAG_PS_CONFIGURATION_NAME flag is not set.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.

#### NOW_EXEC_PWSH_MSG

The NOW_EXEC_PWSH_MSG message is used to execute a remote PowerShell 7 (pwsh) command.

<table class="byte-layout">
    <thead>
        <tr>
            <th>0</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th><th>6</th><th>7</th>
            <th>8</th><th>9</th><th>10</th><th>1</th><th>2</th><th>3</th><th>4</th><th>5</th>
            <th>6</th><th>7</th><th>8</th><th>9</th><th>20</th><th>1</th><th>2</th><th>3</th>
            <th>4</th><th>5</th><th>6</th><th>7</th><th>8</th><th>9</th><th>30</th><th>1</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td colspan="32">msgSize</td>
        </tr>
        <tr>
            <td colspan="8">msgClass</td>
            <td colspan="8">msgType</td>
            <td colspan="16">msgFlags</td>
        </tr>
        <tr>
            <td colspan="32">sessionId</td>
        </tr>
        <tr>
            <td colspan="32">command (variable)</td>
        </tr>
        <tr>
            <td colspan="32">executionPolicy (variable)</td>
        </tr>
        <tr>
            <td colspan="32">configurationName (variable)</td>
        </tr>
    </tbody>
</table>

**msgSize (4 bytes)**: The message size, excluding the header size (8 bytes).

**msgClass (1 byte)**: The message class (NOW_EXEC_MSG_CLASS_ID).

**msgType (1 byte)**: The message type (NOW_EXEC_PWSH_MSG_ID).

**msgFlags (2 bytes)**: The message flags, specifying the PowerShell command-line arguments, same as with NOW_EXEC_WINPS_MSG.

**sessionId (4 bytes)**: A 32-bit unsigned integer containing a unique remote execution session id.

**executionPolicy (variable)**: A NOW_VARSTR structure, same as with NOW_EXEC_WINPS_MSG.

**configurationName (variable)**: A NOW_VARSTR structure, same as with NOW_EXEC_WINPS_MSG.

**command (variable)**: A NOW_VARSTR structure containing the command to execute.
