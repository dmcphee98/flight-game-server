#!/bin/bash
# Updates the Route 53 DNS A record with the instance's current public IP.
# Run once on boot via update-dns.service, before Caddy starts.
# {{HOSTED_ZONE_ID}} and {{RECORD_NAME}} are replaced at deploy time.

# Exit immediately if any command fails
set -euo pipefail

# The EC2 instance metadata service (IMDS) is an HTTP endpoint available at
# 169.254.169.254 on every EC2 instance, exposing information about the instance.
# IMDSv2 requires fetching a short-lived token first, which is then used to
# authenticate subsequent metadata requests.
TOKEN=$(curl -s -X PUT "http://169.254.169.254/latest/api/token" \
    -H "X-aws-ec2-metadata-token-ttl-seconds: 60")
PUBLIC_IP=$(curl -s \
    -H "X-aws-ec2-metadata-token: $TOKEN" \
    http://169.254.169.254/latest/meta-data/public-ipv4)

# Create or update the A record in Route 53.
# TTL of 30 seconds ensures DNS propagates quickly after a reboot.
aws route53 change-resource-record-sets \
    --hosted-zone-id {{HOSTED_ZONE_ID}} \
    --change-batch "{
        \"Changes\": [{
            \"Action\": \"UPSERT\",
            \"ResourceRecordSet\": {
                \"Name\": \"{{RECORD_NAME}}\",
                \"Type\": \"A\",
                \"TTL\": 30,
                \"ResourceRecords\": [{
                    \"Value\": \"$PUBLIC_IP\"
                }]
            }
        }]
    }"