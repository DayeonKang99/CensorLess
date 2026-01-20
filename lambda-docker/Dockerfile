FROM public.ecr.aws/lambda/nodejs:20
COPY index.js ${LAMBDA_TASK_ROOT}
WORKDIR ${LAMBDA_TASK_ROOT}
COPY package*.json ${LAMBDA_TASK_ROOT}

RUN npm i 
RUN npm install axios
RUN npm install https
RUN npm install stream
RUN npm install aws-sdk

CMD [ "index.handler" ]
