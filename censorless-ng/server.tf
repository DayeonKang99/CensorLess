terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

variable "region" {
  type        = string
  description = "AWS region"
}

variable "profile" {
  type        = string
  description = "AWS credential profile"
}

provider "aws" {
  region  = var.region
  profile = var.profile
}

# Use official NixOS AMI
data "aws_ami" "nixos_arm64" {
  owners      = ["427812963091"]
  most_recent = true

  filter {
    name   = "name"
    values = ["nixos/25.05*"]
  }
  filter {
    name   = "architecture"
    values = ["arm64"]
  }
}

# Get the default VPC
data "aws_vpc" "default" {
  default = true
}

# Security group for censorless-ng server
resource "aws_security_group" "censorless_server" {
  name        = "censorless-server"
  description = "Security group for censorless-ng server"
  vpc_id      = data.aws_vpc.default.id

  # Allow SSH
  ingress {
    from_port   = 22
    to_port     = 22
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
  }

  # Allow port 1337 from anywhere
  ingress {
    from_port   = 1337
    to_port     = 1337
    protocol    = "tcp"
    cidr_blocks = ["0.0.0.0/0"]
    description = "Allow censorless server port from anywhere"
  }

  # Allow all outbound
  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }

  tags = {
    Name = "censorless-server"
  }
}

# SSH key pair
resource "aws_key_pair" "censorless" {
  key_name   = "censorless-server-key"
  public_key = file("~/.ssh/aws/pdsec/id_ed25519.pub")
}

# NixOS instance
resource "aws_instance" "censorless_server" {
  ami           = data.aws_ami.nixos_arm64.id
  instance_type = "t4g.micro" # 2cpu, 1gb ram (0.5g ram(nano) was a bit too low for handling the deployment)
  key_name      = aws_key_pair.censorless.key_name

  vpc_security_group_ids = [aws_security_group.censorless_server.id]

  # Increase root volume size
  root_block_device {
    volume_size = 30  # GB
    volume_type = "gp3"
    delete_on_termination = true
  }

  tags = {
    Name = "censorless-server"
  }

  # Ensure instance has a public IP
  associate_public_ip_address = true
}

# Output the instance IP for SSH deployment
output "server_ip" {
  value = aws_instance.censorless_server.public_ip
}

output "server_id" {
  value = aws_instance.censorless_server.id
}
