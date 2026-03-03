# How to install Serverless Bridge
We will use ARM64 architecture because this architecture is the cheapest among the architectures Lambda supports.

### Prerequisite
1. install docker
2. install AWS CLI
3. download or clone this directory

### How to set up
1. create AWS ECR repo
2. issue below commands in terminal:
    1. `cd <to serverless-bridge directory>`
    2. `aws ecr get-login-password --region <region> | docker login --username AWS --password-stdin <ECR URL>`
    3. `docker build --provenance=false --platform linux/arm64 -t <image_name> .`
    4. `docker tag <image_name>:latest <ECR repo URL>:latest`
    5. `docker push <image tag>`
3. create the AWS Lambda function using this AWS ECR
    1. general configs: 128 MB memory, 512 ephemeral storage, timeout 15 seconds
4. create the Lambda function URL with Auth type 'None' and Invoke mode 'RESPONSE_STREAM'
5. set up the Tags 
    1. create  `new-lambda-url` key without value.
