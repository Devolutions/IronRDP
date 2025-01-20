CERT_KEY=server-key.pem
CERT_FILE=server-cert.pem
DAYS=365
RSA_BITS=2048
SUBJECT=/C=US/ST=Test/L=Test/O=Test/OU=Test/CN=localhost

.PHONY: all clean certs

all: $(CERT_KEY) $(CERT_FILE)

$(CERT_KEY) $(CERT_FILE):
	openssl req -x509 -nodes -days $(DAYS) -newkey rsa:$(RSA_BITS) \
		-keyout $(CERT_KEY) -out $(CERT_FILE) \
		-subj "$(SUBJECT)"

clean:
	rm -f $(CERT_KEY) $(CERT_FILE)
