#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "native-oracle.h"
#include <midles.h>

void* __RPC_USER midl_user_allocate(size_t n) { return calloc(1, n); }
void __RPC_USER midl_user_free(void* p) { free(p); }

typedef void (*encode_fn)(handle_t, void*);

static void emit(encode_fn encode, void* object) {
    handle_t h;
    char* buffer = NULL;
    unsigned long size = 0;
    RPC_STATUS status = MesEncodeDynBufferHandleCreate(&buffer, &size, &h);
    if (status != RPC_S_OK) {
        fprintf(stderr, "serialization handle failed: %lu\n", status);
        exit(1);
    }
    encode(h, object);
    for (unsigned long i = 0; i < size; ++i) printf("%02x", (unsigned char)buffer[i]);
    printf("\n");
    MesHandleFree(h);
    free(buffer);
}

int main(void) {
    REQUEST input = {0};
    PREQUEST pinput = &input;
    input.CallId = Negotiate;
    input.Version.version = 0x12345678;
    emit((encode_fn)PREQUEST_Encode, &pinput);

    OUTPUT output = {0};
    POUTPUT poutput = &output;
    output.CallId = Negotiate;
    output.Status = -123;
    output.Version.version = 0x12345678;
    emit((encode_fn)POUTPUT_Encode, &poutput);

    byte data[] = {1, 2, 3};
    KEY key = {0};
    key.reserved2 = -123;
    key.length = 3;
    key.data = data;
    hyper skew = -123456789;
    input.CallId = Authenticator;
    input.Auth.key = &key;
    input.Auth.skew = &skew;
    emit((encode_fn)PREQUEST_Encode, &pinput);
    key.reserved1 = data;
    emit((encode_fn)PREQUEST_Encode, &pinput);

    unsigned long count = 3;
    byte* ptr = data;
    output.CallId = Hash;
    output.Checksum.ChecksumSize = &count;
    output.Checksum.ChecksumValue = &ptr;
    emit((encode_fn)POUTPUT_Encode, &poutput);

    NREQUEST ntinput = {0};
    PNREQUEST pntinput = &ntinput;
    ntinput.CallId = NtlmNegotiate;
    ntinput.Version.version = 0x12345678;
    emit((encode_fn)PNREQUEST_Encode, &pntinput);

    NOUTPUT ntoutput = {0};
    PNOUTPUT pntoutput = &ntoutput;
    ntoutput.CallId = NtlmCompare;
    ntoutput.Status = -123;
    ntoutput.Compare.nt = -1;
    ntoutput.Compare.lm = 1;
    ntoutput.Compare.sha = 0;
    emit((encode_fn)PNOUTPUT_Encode, &pntoutput);
    return 0;
}
