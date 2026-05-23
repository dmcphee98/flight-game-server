"""
Defines the AWS infrastructure for the flight game server.
Provisions a VPC, EC2 instance, Route 53 DNS record, and IAM roles,
and configures the instance via user data to run the game server and Caddy.
"""

import os
from aws_cdk import (
    Stack,
    Duration,
    aws_ec2 as ec2,
    aws_iam as iam,
    aws_route53 as route53,
)
from constructs import Construct

def load_script(filename, replacements={}):
    """
    Loads a script from the scripts/ directory and replaces any {{PLACEHOLDER}}
    tokens with the provided values. Used to inject CDK context values (hosted
    zone ID, subdomain, port, etc.) into shell scripts and systemd unit files
    before they are written to the EC2 instance via user data.
    """
    dir = os.path.dirname(os.path.abspath(__file__))
    with open(os.path.join(dir, "scripts", filename)) as f:
        content = f.read()
    for key, value in replacements.items():
        content = content.replace(f"{{{{{key}}}}}", value)
    return content

class CdkStack(Stack):

    def __init__(self, scope: Construct, construct_id: str, **kwargs) -> None:
        super().__init__(scope, construct_id, **kwargs)

        # Required context values when calling cdk synth or cdk deploy
        domain = self.node.try_get_context("domain")
        subdomain = self.node.try_get_context("subdomain")
        port = self.node.try_get_context("port")

        if not all([domain, subdomain, port]):
            raise ValueError(
                "Missing required context values. Pass them in with the -c flag:\n"
                "  -c domain=example.com -c subdomain=game -c port=8080"
            )

        # --- VPC ---
        vpc = ec2.Vpc(self, "FlightGameVpc",
            max_azs=1,                                # No need for multi-AZ redundancy.
            nat_gateways=0,                           # No NAT gateway required for public subnets.
            subnet_configuration=[
                ec2.SubnetConfiguration(
                    name="Public",
                    subnet_type=ec2.SubnetType.PUBLIC,
                    cidr_mask=24,
                )
            ]
        )

        # --- Hosted Zone ---

        # Get existing hosted zone from Route53
        zone = route53.HostedZone.from_lookup(self, "Zone", domain_name=domain)

        # Create A record in hosted zone, which will ultimately point to the EC2 instance
        route53.ARecord(self, "FlightGameDnsRecord",
            zone=zone,
            record_name="flightgame",
            target=route53.RecordTarget.from_ip_addresses("1.2.3.4"),  # placeholder, overwritten on EC2 boot
            ttl=Duration.seconds(30),
        )

        # --- Security Group ---
        sg = ec2.SecurityGroup(self, "FlightGameSg",
            vpc=vpc,
            description="Flight game server security group",
            allow_all_outbound=True,
        )
        sg.add_ingress_rule(ec2.Peer.any_ipv4(), ec2.Port.tcp(80),   "HTTP (Caddy redirect)")
        sg.add_ingress_rule(ec2.Peer.any_ipv4(), ec2.Port.tcp(443),  "HTTPS / WSS")

        # --- IAM Role ---
        role = iam.Role(self, "FlightGameInstanceRole",
            assumed_by=iam.ServicePrincipal("ec2.amazonaws.com"),
        )

        # --- EC2 User Data ---

        replacements = {
            "HOSTED_ZONE_ID": zone.hosted_zone_id,
            "RECORD_NAME": f"{subdomain}.{domain}",
            "GAME_SERVER_PORT": port,
        }
        update_dns_sh = load_script("update-dns.sh", replacements)
        update_dns_service = load_script("update-dns.service")
        game_server_service = load_script("game-server.service")

        user_data = ec2.UserData.for_linux()
        user_data.add_commands(
            # Install Docker
            "dnf install -y docker",
            "systemctl enable docker",
            "systemctl start docker",

            # Write Caddyfile before starting container
            "mkdir -p /etc/caddy",
            f"cat > /etc/caddy/Caddyfile << 'SCRIPT'\n{subdomain}.{domain} {{\n    reverse_proxy localhost:{port}\n}}\nSCRIPT",

            # Pull and run Caddy
            "docker pull caddy",
            "docker run -d --name caddy --restart always \
                -p 80:80 -p 443:443 \
                -v /etc/caddy/Caddyfile:/etc/caddy/Caddyfile \
                -v caddy_data:/data \
                caddy",

            # update-dns script
            f"cat > /usr/local/bin/update-dns.sh << 'SCRIPT'\n{update_dns_sh}\nSCRIPT",
            "chmod +x /usr/local/bin/update-dns.sh",

            # systemd units
            f"cat > /etc/systemd/system/update-dns.service << 'SCRIPT'\n{update_dns_service}\nSCRIPT",
            f"cat > /etc/systemd/system/game-server.service << 'SCRIPT'\n{game_server_service}\nSCRIPT",

            # Enable all services (systemd will start these services on every boot in correct order)
            "systemctl daemon-reload",
            "systemctl enable update-dns.service",
            "systemctl enable game-server.service",

            "systemctl start update-dns.service", # run immediately on first boot
        )

        # --- EC2 ---
        instance = ec2.Instance(self, "FlightGameInstance",
            instance_type=ec2.InstanceType("t4g.micro"),
            machine_image=ec2.MachineImage.latest_amazon_linux2023(
                cpu_type=ec2.AmazonLinuxCpuType.ARM_64,
            ),
            vpc=vpc,
            vpc_subnets=ec2.SubnetSelection(subnet_type=ec2.SubnetType.PUBLIC),
            security_group=sg,
            role=role,
            associate_public_ip_address=True,
            user_data=user_data,
        )

        # Permit EC2 instance to update A record in Route53 on boot.
        # Opted not to use Elastic IP address, so EC2 IP will change with every restart.
        # A record must be updated to point to this new IP each time.
        role.add_to_policy(iam.PolicyStatement(
            actions=["route53:ChangeResourceRecordSets"],
            resources=[zone.hosted_zone_arn],
        ))

        # Permit EC2 instance to stop itself after the last player leaves
        role.add_to_policy(iam.PolicyStatement(
            actions=["ec2:StopInstances"],
            resources=["*"],
            conditions={
                "StringEquals": {"ec2:ResourceTag/aws:cloudformation:stack-name": self.stack_name}
            }
        ))

        # Permit connections to EC2 instance via AWS Systems Manager
        # Can shell in via `aws ssm start-session` without opening port 22 or managing SSH keys.
        role.add_managed_policy(
            iam.ManagedPolicy.from_aws_managed_policy_name("AmazonSSMManagedInstanceCore")
        )