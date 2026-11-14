#!/bin/sh
# Run a build command trusting an optional extra CA (BuildKit secret "ca"),
# for builds behind TLS-intercepting corporate proxies:
#   docker build --secret id=ca,src=/path/proxy-ca.crt \
#     --build-arg https_proxy=http://proxy:3128 ...
# Without the secret this just runs the command.
set -e
if [ -s /run/secrets/ca ]; then
  # Secret mounts are root-only; apt downloads as the unprivileged _apt user.
  CA=/tmp/extra-ca.crt
  cp /run/secrets/ca "$CA" && chmod 644 "$CA"
  export SSL_CERT_FILE=$CA CARGO_HTTP_CAINFO=$CA NODE_EXTRA_CA_CERTS=$CA GIT_SSL_CAINFO=$CA
  if [ -d /etc/apt ]; then
    echo "Acquire::https::CaInfo \"$CA\";" > /etc/apt/apt.conf.d/99extra-ca
    # Intercepting proxies often allow only HTTPS.
    sed -i -e 's#http://deb.debian.org#https://deb.debian.org#g' -e 's#http://archive.ubuntu.com#https://archive.ubuntu.com#g' \
      -e 's#http://security.ubuntu.com#https://security.ubuntu.com#g' /etc/apt/sources.list.d/*.sources 2>/dev/null || true
  fi
fi
exec "$@"
