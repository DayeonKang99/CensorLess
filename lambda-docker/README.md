# How to install Lambda Proxy
We will use ARM64 architecture because this architecture is the cheapest among the architectures Lambda supports.

### Prerequisite
1. install docker
2. install AWS CLI
3. download or clone this directory

### How to set up
1. create AWS ECR repo
2. issue below commands in terminal:
    1. `cd <to lambda-docker directory>`
    2. `aws ecr get-login-password --region <region> | docker login --username AWS --password-stdin <ECR URL>`
    3. `docker build --provenance=false --platform linux/arm64 -t <image_name> .`
    4. `docker tag <image_name>:latest <ECR repo URL>:latest`
    5. `docker push <image tag>`
3. Now, lambda-manager will automatically install and control the Lambda Proxy. 
