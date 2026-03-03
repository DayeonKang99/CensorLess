from time import sleep
import time
import os
import boto3
import pandas as pd
import numpy as np
import datetime
import urllib.request, json 
import requests
import adal
import math
from http.server import BaseHTTPRequestHandler, HTTPServer
from functools import partial 
from typing import List, Dict
import threading
import sys
from collections import defaultdict 

import rejuvenation

US_REGIONS = ['us-west-1'] #'us-east-1', 'us-east-2', 'us-west-1', 'us-west-2']
clients = {}
region = US_REGIONS[0]
# current_type = 't2.micro'
capacity = 2
INSTANCE_MANAGER_INSTANCE_ID = "i-035f88ca820e399e7"
CLIENT_INSTANCE_ID = "i-0c7adc535b262d69e"
SERVICE_INSTANCE_ID = "i-0dd2ca9d91838f3c8"
# for creating Lambda functions
TIMEOUT = 15
MEMORY_SIZE = 128
EPHEMERAL_STORAGE = 512
# image = "221082198449.dkr.ecr.us-east-1.amazonaws.com/lambdaproxy-ver1-250306:latest"
# role = "arn:aws:iam::221082198449:role/Lambda-template"
# FLEETIDX = 0

def pretty_json(obj):
    return json.dumps(obj, sort_keys=True, indent=4, default=str)

def parse_input_args(filename):
    with open(filename, 'r') as j:
        input_args = json.loads(j.read())
        # Convert to list of keys only:
        # excluded_instances = list(cred_json.values())
        # print(excluded_instances)
    return input_args

def choose_session(region):
    client = boto3.client('lambda', region)
    return client

def chunks(lst, n):
    """
        Breaks a list into equally sized chunks of size n.
        Parameters:
            lst: list
            n: int

        https://stackoverflow.com/a/312464/13336187

        Usage example: list(chunks(range(10, 75), 10)
    """
    """Yield successive n-sized chunks from lst."""
    for i in range(0, len(lst), n):
        yield lst[i:i + n]

'''
def get_instance_type(ec2, types):
    response = ec2.describe_instance_types(
        InstanceTypes=types
    )
    return response
'''

## merge get_all_instances and get_all_running_instances
def get_all_functions(client):
    response = client.list_functions()
    # response['Reservations'][0]['Instances'][0]['InstanceId']
    # print(pretty_json(response))
    function_arns = [func['FunctionArn'] for func in extract_function_details_from_describe_functions_response(response)]
    return function_arns

def extract_function_details_from_describe_functions_response(response):
    """
        Purpose: each element of the response['Reservations'] list only holds 25 instances. 
    """
    function_list = []
    for i in response:
        function_list.append(i['Functions'])
    return function_list

## control excluded_functions?

def get_all_functions_url(client, excluded_functions):
    instances_details = defaultdict(dict)
    response = client.list_functions()
    for func in extract_function_details_from_describe_functions_response(response):
        # print(instance['InstanceId'])
        if func['FunctionArn'] not in excluded_functions: # no need to include instance manager since we will not assign clients to it anyway..
            function_url = client.list_function_url_configs(FunctionName=func['FunctionArn'])
            instances_details[func['FunctionArn']] = {"FunctionUrl": function_url['FunctionUrlConfigs'][0]['FunctionUrl']}
    # instance_ids = [instance['InstanceId'] for instance in response['Reservations'][0]['Instances']]
    return instances_details

def get_excluded_terminate_instances():
    # Get excluded from termination instance list:
    excluded_instances = []
    with open("misc/exclude-from-termination-list.json", 'r') as j:
        cred_json = json.loads(j.read())
        # Convert to list of keys only:
        excluded_instances = list(cred_json.values())
        # print(excluded_instances)
    return excluded_instances

# def get_all_instances_init_details(ec2):
#     """
#         Used only for wireguard integration only for now: GET endpoint
#     """
#     response = ec2.describe_instances(
#         Filters=[
#             {
#                 'Name': 'instance-state-name',
#                 'Values': [
#                     'running',
#                 ]
#             }
#         ]
#     )
#     # Get excluded from termination instance list:
#     # excluded_instances = get_excluded_terminate_instances()
#     # response['Reservations'][0]['Instances'][0]['InstanceId']
#     return extract_init_details_from_describe_instances_response(response, excluded_instances)

# def extract_init_details_from_describe_instances_response(response, excluded_instances):
#     instances_details = defaultdict(dict)
#     for instance in extract_instance_details_from_describe_instances_response(response):
#         # print(instance['InstanceId'])
#         if instance['InstanceId'] not in excluded_instances: # no need to include instance manager since we will not assign clients to it anyway..
#             instances_details[instance['InstanceId']] = {"PublicIpAddress": instance['PublicIpAddress']}
#     # instance_ids = [instance['InstanceId'] for instance in response['Reservations'][0]['Instances']]
#     return instances_details

# def get_specific_instances_attached_ebs(ec2, instance_id):
#     """
#         Get an instance's attached NIC EBS volume details. 
#     """
    
#     volumes = ec2.describe_instance_attribute(InstanceId=instance_id,
#         Attribute='blockDeviceMapping')
#     # Get ec2 instance attached NIC IDs:
#     # nics = ec2.describe_instance_attribute(InstanceId=instance_id,
#     #     Attribute='networkInterfaceSet')
#     return volumes 


def get_specific_functions(client, function_arn):
    response = client.get_function(
        FunctionName=function_arn
    )
    return response

def get_specific_functions_with_fleet_id_tag(client, region, fleet_id, return_type="init-details"):
    """
        tag:<key> - The key/value combination of a tag assigned to the resource. Use the tag key in the filter name and the tag value as the filter value. For example, to find all resources that have a tag with the key Owner and the value TeamA, specify tag:Owner for the filter name and TeamA for the filter value.

        Parameters:
            - return_type: "raw" | "init-details"
    """
    resourceAPI = boto3.client('resourcegroupstaggingapi', region)
    response = resourceAPI.get_resources(
        TagFilters=[
            {
                'Key': 'FleetID',
                'Values': [
                    fleet_id
                ]
            }
        ],
        ResourceTypeFilters=[
        'lambda:function',
        ]
    )
    with open("response.json", "w") as f:
        print(pretty_json(response), file=f)
    # instance_details = {}
    # for instance in response['Reservations'][0]['Instances']:
    #     instance_details[instance['InstanceId']] = {}
    # instance_ids = [instance['InstanceId'] for instance in response['Reservations'][0]['Instances']]
    functionARNs = []
    for i in response['ResourceTagMappingList']:
        functionARNs.append(i['ResourceARN'])
        
    if return_type == "raw":
        functions_details = []
        for i in functionARNs:
            r = get_specific_functions(client, i)
            functions_details.append(r['Configuration'])
        return functions_details #extract_instance_details_from_describe_instances_response(response) # quite a complex dict, may need to prune out useless information later
    else: 
        excluded_instances = get_excluded_terminate_instances()
        # response['Reservations'][0]['Instances'][0]['InstanceId']
        functions_details = defaultdict(dict)
        for func in functionARNs:
            # print(instance['InstanceId'])
            if func not in excluded_instances: # no need to include instance manager since we will not assign clients to it anyway..
                function_url = client.list_function_url_configs(FunctionName=func)
                functions_details[func] = {"FunctionUrl": function_url['FunctionUrlConfigs'][0]['FunctionUrl']}
        # instance_ids = [instance['InstanceId'] for instance in response['Reservations'][0]['Instances']]
        return functions_details

# def get_all_active_spot_fleet_requests(ec2):
#     response = ec2.describe_spot_fleet_requests()
#     print(response)
#     # Filter for active (running) spot fleet requests
#     active_fleet_requests = [fleet['SpotFleetRequestId'] 
#                             for fleet in response['SpotFleetRequestConfigs'] 
#                             if fleet['SpotFleetRequestState'] in ['active', 'modifying']]
#     return active_fleet_requests

# def start_instances(ec2, instance_ids):
#     response = ec2.start_instances(
#         InstanceIds=instance_ids
#     )
#     return response

# def stop_instances(ec2, instance_ids):
#     response = ec2.stop_instances(
#         InstanceIds=instance_ids
#     )
#     return response

# def reboot_instances(ec2, instance_ids):
#     response = ec2.reboot_instances(
#         InstanceIds=instance_ids
#     )
#     return response

def delete_functions(client, function_arn):
    response = client.delete_function(
        FunctionName=function_arn
    )
    return response

def nuke_all_functions(client, excluded_functions):
    """
        Terminates all instances, and spot requests except for the ones specified in excluded_instance_ids
    """
    functions = get_all_functions(client)
    functions_to_terminate = []
    for func in functions:
        if func not in excluded_functions:
            # print(instance)
            functions_to_terminate.append(func)
    if len(functions_to_terminate) > 300:
        # We can only terminate 300 instances at a time (I believe..)
        for chunk in chunks(functions_to_terminate, 300):
            response = delete_functions(client, chunk)
            print(response)
    else:
        response = delete_functions(client, functions_to_terminate)
        print(response)
    # response = terminate_instances(ec2, instances_to_terminate)
    # print(response)

    # print(get_all_active_spot_fleet_requests(ec2))
    # response = ec2.cancel_spot_fleet_requests(
    #     SpotFleetRequestIds=get_all_active_spot_fleet_requests(ec2),
    #     TerminateInstances=True
    # )

    return functions_to_terminate

def create_function(client, region, image, role, tag):
    function_name = 'lambdaproxy-'+str(round(time.time()))
    print(f"creating Lambda function named '{function_name}' in region {region}")
    response_func = client.create_function(
        FunctionName=function_name,
        Role=role,
        Code={
            'ImageUri': image
        },
        Timeout=TIMEOUT,
        MemorySize=MEMORY_SIZE,
        PackageType='Image',
        Architectures=[
            'arm64',
        ],
        EphemeralStorage={
            'Size': EPHEMERAL_STORAGE
        }
    )
    function_arn = response_func['FunctionArn']
    response = client.create_function_url_config(
        FunctionName=function_name,
        AuthType='NONE', # | 'AWS_IAM'
        # Cors = {} # The cross-origin resource sharing (CORS) settings for your function URL.
        InvokeMode='RESPONSE_STREAM'
    )
    response = client.add_permission(
        Action='lambda:InvokeFunctionUrl',
        FunctionName=function_name,
        Principal='*',
        StatementId='FunctionURLAllowPublicAccess',
        FunctionUrlAuthType='NONE'
    )
    tag_response = assign_name_tags(client, function_arn, tag)

    return response

# def get_addresses(ec2):
#     response = ec2.describe_addresses()
#     return response

# def get_public_ip_address(ec2, eip_id):
#     response = ec2.describe_addresses(
#         AllocationIds=[
#             eip_id,
#         ]
#     )
#     return response['Addresses'][0]['PublicIp']

# def allocate_address(ec2):
#     response = ec2.allocate_address(
#         Domain='vpc'
#     )
#     return response

# def get_eip_id_from_allocation_response(response):
#     """
#         Returns the EIP ID from the response of allocate_address
#     """
#     return response['AllocationId']

# def release_address(ec2, allocation_id):
#     response = ec2.release_address(
#         AllocationId=allocation_id
#     )
#     return response

# def associate_address(ec2, instance_id, allocation_id, network_interface_id):
#     response = ec2.associate_address(
#         # InstanceId=instance_id,
#         AllocationId=allocation_id,
#         NetworkInterfaceId=network_interface_id
#     )
#     return response

# def get_association_id_from_association_response(response):
#     return response['AssociationId']

# def disassociate_address(ec2, association_id):
#     response = ec2.disassociate_address(
#         AssociationId=association_id
#     )
#     return response    

def assign_name_tags(client, resource_id, name):
    responses = []
    if not isinstance(resource_id, list):
        resource_id = [resource_id]
    for r_id in resource_id:
        response = client.tag_resource(
            Resource=r_id, # resource could be an instance, network interface, eip, etc
            Tags={
                    'FleetID': name,
                    'new-lambda-url': ''
                }
        )
        responses.append(response)
    return responses

#maybe ping doesn't work. this method is for the test
# def ping(ip, backoff_time, trials):
#     """
#         Parameters:
#             ip: string
#             backoff_time: int # seconds
#             trials: int
#         Returns:
#             True if ping is successful, False otherwise
#     """
#     for i in range(trials):
#         response = os.system("ping -c 1 " + ip)
#         if response == 0:
#             return 0
#         else:
#             time.sleep(backoff_time)
#     return 1

# def ping_instances(ec2, nic_list, multi_NIC=True, not_fixed=True):
#     """
#         Checks if instances are pingable.

#         Parameters:
#             nic_list: list of NIC IDs
#             not_fixed: True | False
#                 - True: # TODO Minor quirk that will be removed later: only the default NIC (i.e., original_nic) is configured to accept pings for now. We will need to fix this later. Will remove this parameter altogether once fixed. 
#     """
#     failed_ips = []

#     # Retry details:
#     backoff_time = 10 # seconds
#     trials = 3

#     # time.sleep(wait_time)
#     if not_fixed: # only ping the original NIC
#         nic_details = nic_list[-1] # this is the position of the original_nic, since we append it last..
#         ip = nic_details[-1]
#         response = ping(ip, backoff_time, trials)
#         if response == 0:
#             print(f"{ip} is up!")
#         else:
#             print(f"{ip} is down!")
#             # if ping fails, add to failed_ips
#             failed_ips.append(ip)
#     else: # ping all NICs
#         for nic_details in nic_list:
#             ip = nic_details[-1]
#             response = ping(ip, backoff_time, trials)
#             if response == 0:
#                 print(f"{ip} is up!")
#             else:
#                 print(f"{ip} is down!")
#                 # if ping fails, add to failed_ips
#                 failed_ips.append(ip)
#     return failed_ips

def create_initial_fleet_and_periodic_rejuvenation_thread(client, input_args, quick_test=False):

    # Extract required input args:
    REJUVENATION_PERIOD = int(input_args['REJUVENATION_PERIOD']) # in seconds
    regions = input_args['regions']
    PROXY_COUNT = int(input_args['PROXY_COUNT']) # aka size of lambda function 
    # PROXY_IMPL = input_args['PROXY_IMPL'] # wireguard | snowflake
    batch_size = input_args['batch_size'] # number of instances to create per thread. Currently, we assume this is completely divisible by PROXY_COUNT.
    # MIN_COST = float(input_args['MIN_COST'])
    # MAX_COST = float(input_args['MAX_COST'])
    # MIN_VCPU = int(input_args['MIN_VCPU']) # not used for now
    # MAX_VCPU = int(input_args['MAX_VCPU']) # not used for now
    INITIAL_EXPERIMENT_INDEX = int(input_args['INITIAL_EXPERIMENT_INDEX'])
    # multi_nic = input_args['multi_NIC'] # boolean
    # mode = input_args['mode'] # liveip | instance 
    data_dir = input_args['dir'] # used for placing the logs.
    wait_time_after_create = input_args['wait_time_after_create'] # e.g., 30
    # wait_time_after_nic = input_args['wait_time_after_nic'] # e.g., 30

    filter = {
        # "min_cost": MIN_COST, #0.002 for first round of exps..
        # "max_cost": MAX_COST,
        "regions": regions
    }

    launch_templates = []

    # if PROXY_IMPL == 'snowflake' or PROXY_IMPL == 'wireguard':
    #     # launch_templates.extend([input_args['launch-template-main'], input_args['launch-template-side'], input_args['launch-template-peer']]) # main: is the first (and is a single) proxy to connect to, and peer is a client. side is the rest of the proxies. TODO: add creation of a single main and peer later here in this script. 
    #     launch_templates.append(input_args['launch-template'])
    # else:
    #     raise Exception("Invalid proxy implementation: " + PROXY_IMPL)  

    # # Get cheapest instance:
    # prices = update_spot_prices(ec2)
    # prices = prices.sort_values(by=['SpotPrice'], ascending=True)
    # # print(prices.iloc[0])
    # index, cheapest_instance = get_instance_row_with_supported_architecture_and_regions(ec2, prices, regions=regions)
    # instance_type = cheapest_instance['InstanceType']
    # zone = cheapest_instance['AvailabilityZone']
    
    # Create fleet by batch:
    batch_count = math.ceil(PROXY_COUNT/batch_size)
    initial_region = "us-east-1" # used for initialization purposes ## do we have to change this..?
    threads = []
    for i in range(batch_count):
        # if mode == "instance":
        tag_prefix = "instance-exp{}-{}fleet".format(str(INITIAL_EXPERIMENT_INDEX), str(PROXY_COUNT))
        filename = data_dir + tag_prefix + "-batch-count-{}".format(i) + ".txt"
        file = open(filename, 'w+')
        rejuvenator = rejuvenation.FunctionRejuvenator(initial_region, input_args, filter, tag_prefix, filename)
            
        # elif mode == "liveip":
        #     tag_prefix = "liveip-exp{}-{}fleet-{}mincost".format(str(INITIAL_EXPERIMENT_INDEX), str(PROXY_COUNT), str(filter['min_cost']))
        #     filename = data_dir + tag_prefix + "-batch-count-{}".format(i) + ".txt"
        #     file = open(filename, 'w+')ç
        #     # live_ip_rejuvenation(initial_ec2, is_UM, REJUVENATION_PERIOD, PROXY_COUNT, EXPERIMENT_DURATION, PROXY_IMPL, filter=filter, tag_prefix=tag_prefix, wait_time_after_create=wait_time_after_create, print_filename=filename)
        #     rejuvenator = rejuvenation.LiveIPRejuvenator(initial_region, launch_templates, input_args, filter, tag_prefix, filename)

        thread = threading.Thread(target=rejuvenator.rejuvenate, kwargs={
            "quick_test": quick_test
        })
        thread.start()
        threads.append(thread)

    return threads

class RequestHandler(BaseHTTPRequestHandler):
    def __init__(self, client, input_args, *args, **kwargs):
        self.client = client
        super().__init__(*args, **kwargs)

    def _set_response(self):
        self.send_response(200)
        self.send_header('Content-type', 'text/html')
        self.end_headers()

    def do_GET(self):
        path = self.path.split('/')[1:]
        print(path)
        match path[0]:
            case 'getNum':
                instances = get_all_functions(self.client)
                num = len(instances)
                self._set_response()
                self.wfile.write(str(num).encode('utf-8'))
            case 'interrupt':
                id = path[1]
                response = delete_functions(self.client, id)
                create_function(self.client, region, rejuvenation.image, rejuvenation.role, rejuvenation.tag)
                self._set_response()
                self.wfile.write(response.encode('utf-8'))
                #notices controller to interrupt instance, WIREGUARD ONLY
            case "getInitDetails":
                # print("Enter getInitDetails")
                instances_details = get_all_functions(self.client)
                self._set_response()
                self.wfile.write(pretty_json(instances_details).encode('utf-8'))
            # case "createWireguardMain": # hardcoded. only for the convenience of artifact evaluation.
            #     response = create_fleet(self.client, "m7a.large", "us-east-1", input_args["launch-template-main"], 1)
            #     self._set_response()
            #     self.wfile.write(response.encode('utf-8'))
            #     # TODO: respond to controller..
            # case "createWireguardClient": # hardcoded. only for the convenience of artifact evaluation. 
            #     # print("Enter createClient")
            #     response = create_fleet(self.client, "m7a.medium", "us-east-1", input_args["launch-template-peer"], 1) 
            #     self._set_response()
            #     self.wfile.write(response.encode('utf-8'))
            # case "createSnowflakeClient": # TODO: uncomment later. not working yet. Probably need to figure out how to do this with Jinyu separately
            #     # print("Enter createClient")
            #     response = create_fleet(self.ec2, "m7a.medium", "us-east-1", input_args["launch-template-peer"], 1) 
            #     self._set_response()
            #     self.wfile.write(response.encode('utf-8'))
            # case "createNAT": # TODO: uncomment later. not working yet. Probably need to figure out how to do this with Sina and Jinyu separately
            #     # print("Enter createNAT")
            #     response = create_fleet(self.ec2, current_type, region, launch_template, 1)
            #     self._set_response()
            #     self.wfile.write(response.encode('utf-8'))

def run_server(client):
    server_address = ('', 6000)
    # https://stackoverflow.com/questions/21631799/how-can-i-pass-parameters-to-a-requesthandler
    handler = partial(RequestHandler, client)
    httpd = HTTPServer(server_address, handler)
    print('Starting server...')
    httpd.serve_forever()

def get_instance_row_with_supported_architecture(client, prices, supported_architecture=['x86_64']):
    """
        Copied from rejuvenation-eval-script.py
        Parameters:
            supported_architecture: list of architectures to support. Default is x86_64 (i.e., Intel/AMD)
            prices: df of prices (from get_cheapest_instance_types_df)
        Returns:
            row of the cheapest instance type that supports the architecture
    """
    for index, row in prices.iterrows():
        instance_type = row['InstanceType']
        instance_info = get_instance_type(client, [instance_type])
        for arch in instance_info['InstanceTypes'][0]['ProcessorInfo']['SupportedArchitectures']:
            if arch in supported_architecture:
                return index, row
        # if instance_info['InstanceTypes'][0]['ProcessorInfo']['SupportedArchitectures'][0] in supported_architecture:
        #     return index, row
    raise Exception("No instance type supports the architecture: " + str(supported_architecture))

# example usage of creating 2 instances in us-east-1 with UM account: python3 api.py UM us-east-1 2 main
# explanation of above example: this creates 2 instances in the us-east-1a az, in the UM AWS account
if __name__ == '__main__':
    # FLEETIDX = 0
    input_args_filename = sys.argv[1]
    input_args = parse_input_args(input_args_filename)

    if len(sys.argv) > 2:
        if sys.argv[2] == "simple-test":
            client = choose_session(region=region)
            threads = create_initial_fleet_and_periodic_rejuvenation_thread(client, input_args, quick_test=True)
        else:
            # Exit script, invalid argument:
            print("Invalid argument: " + sys.argv[2])
            sys.exit(1)
    else:
        client = choose_session(region=region)
        threads = create_initial_fleet_and_periodic_rejuvenation_thread(client, input_args)

        run_server(client)

    time.sleep(10) # wait for threads to start

    for thread in threads:
        # Wait for threads to end:
        thread.join()

    # Some example usage from Patrick:
    """
    response = get_all_instances()

    # Create two instances: 
    UM_launch_template_id = "lt-07c37429821503fca"
    response = create_fleet("t2.micro", "us-east-1c", UM_launch_template_id, 2) # verified working (USE THIS)

    response = create_fleet2("t2.micro", "us-east-1c", UM_launch_template_id, 2) # not working yet

    print(response)

    # Delete instances using the fleet-id key returned from the response above:

    instance_ids = get_specific_instances_with_fleet_id_tag('fleet-4da19c85-1000-4883-a480-c0b7a34b444b')
    print(instance_ids)
    for i in instance_ids:
        response = terminate_instances([i])
        print(response)
    """