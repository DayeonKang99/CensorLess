import api as api
import time
import math
import json
import requests
import random
import warnings

# image = "221082198449.dkr.ecr.us-east-1.amazonaws.com/lambdaproxy-ver1-250306:latest"
image = "221082198449.dkr.ecr.us-west-1.amazonaws.com/lambdaproxy-streaming:latest"
role = "arn:aws:iam::221082198449:role/Lambda-template"
FLEETIDX = 0
# tag = 'fleet-'+str(FLEETIDX)

class Rejuvenator:
    """Abstract class for rejuvenators."""

    def __init__(self, initial_region, input_args, filter, tag_prefix, print_filename) -> None:
        """Initializes the Rejuvenator object."""

        self.input_args = input_args
        self.initial_region = initial_region
        # self.launch_templates = launch_templates
        self.filter = filter ##
        self.tag_prefix = tag_prefix ##
        self.print_filename = print_filename

        # Extract required input args:
        self.REJUVENATION_PERIOD = int(input_args['REJUVENATION_PERIOD']) # in seconds
        self.initial_proxy_url = input_args['initial_proxy_url']
        self.initial_proxy_arn = input_args['initial_proxy_arn']
        self.regions = input_args['regions']
        self.controller_ip = input_args['controller-IP']
        self.PROXY_COUNT = int(input_args['PROXY_COUNT']) # aka fleet size 
        # self.PROXY_IMPL = input_args['PROXY_IMPL'] # wireguard | snowflake
        self.batch_size = input_args['batch_size'] # number of instances to create per thread. Currently, we assume this is completely divisible by PROXY_COUNT.
        # self.MIN_COST = float(input_args['MIN_COST'])
        # self.MAX_COST = float(input_args['MAX_COST'])
        # self.MIN_VCPU = int(input_args['MIN_VCPU']) # not used for now
        # self.MAX_VCPU = int(input_args['MAX_VCPU']) # not used for now
        # self.INITIAL_EXPERIMENT_INDEX = int(input_args['INITIAL_EXPERIMENT_INDEX'])
        # self.multi_nic = input_args['multi_NIC'] # boolean
        # self.mode = input_args['mode'] # liveip | instance 
        # self.data_dir = input_args['dir'] # used for placing the logs.
        self.wait_time_after_create = input_args['wait_time_after_create'] # e.g., 30
        # self.wait_time_after_nic = input_args['wait_time_after_nic'] # e.g., 30

    def print_stdout_and_filename(self, string, filename):
        print(string)
        with open(filename, 'a') as file:
            file.write(string + "\n")

    def extract_urls_and_notify_controller(self, function_list_old, function_list):
        old_urls = []
        new_urls = []
        old_arns = []
        # DEBUG: Print the structures
        print("=" * 80)
        print("DEBUG: function_list structure:")
        print(f"  Type: {type(function_list)}")
        print(f"  Length: {len(function_list)}")
        print(f"  Content: {function_list}")
        
        print("\nDEBUG: function_list_old structure:")
        print(f"  Type: {type(function_list_old)}")
        print(f"  Length: {len(function_list_old)}")
        print(f"  Content: {function_list_old}")
        print("=" * 80)
        for batch in function_list:
            for function_details in batch:
                new_urls.append(function_details['FunctionURL'])
        # for function_details in function_list[0]:
        #     new_urls.append(function_details['FunctionURL'])
            # for url in function_details['FunctionURL']:
            #     new_urls.extend(url[-1])
        if len(function_list_old) != 0:
            for function_details in function_list_old[0]:
                old_urls.append(function_details['FunctionURL'])
                old_arns.append(function_details['FunctionArn'])
                # for url in function_details['FunctionURL']:
                #     old_urls.extend(url[-1])
                # for arn in function_details['FunctionArn']:
                #     old_arns.extend(url[-1])
        else:
            # If no old functions, create empty placeholders to match new_urls length
            old_urls = [""] * len(new_urls)
            old_arns = [""] * len(new_urls)
        print(f"old urls: {old_urls}")
        print(f"new urls: {new_urls}")
        if len(old_urls) != len(new_urls):
            for old_url in old_urls:
                if old_url in new_urls:
                    new_urls.remove(self.initial_proxy_url)
            if self.initial_proxy_url in new_urls:
                new_urls.remove(self.initial_proxy_url)
            if self.initial_proxy_url in old_urls:
                old_urls.remove(self.initial_proxy_url)
        self.notify_controller([old_urls[0]], [old_arns[0]], new_urls)

    def notify_controller(self, old_urls, old_arns, new_urls):
        url = "http://{}:8000/assignments/postsingleupdate".format(self.controller_ip)
        payload = {
            "old_urls": old_urls,
            "old_arns": old_arns,
            "new_urls": new_urls
        }
        print(payload)
        # Send the POST request
        headers = {'Content-Type': 'application/json'}
        response = requests.post(url, data=json.dumps(payload), headers=headers)

        if response.status_code == 200:
            self.print_stdout_and_filename("Request was successful", self.print_filename)
            self.print_stdout_and_filename("Response: {}".format(response.json()), self.print_filename)
            return True
        else:
            self.print_stdout_and_filename("Failed to send POST request", self.print_filename)
            self.print_stdout_and_filename("Status code: {}".format(response.status_code), self.print_filename)
            self.print_stdout_and_filename("Response: {}".format(response.text), self.print_filename)
            # Raise error:
            raise Exception("Failed to send POST request. Status code: {}. Response: {}".format(response.status_code, response.text))

    def create_fleet(self, initial_client):
        """
            Creates the required fleet: 
            Parameters:
                filter: refer to get_cheapest_instance_types_df for definition 
                tag_prefix: will be used to tag all resources associated with this instance 
                multi_NIC == True, used for liveIP and optimal
                wait_time_after_create: in seconds

            Returns:
                List of created instances (count guaranteed to be proxy_count)
                    [
                        {
                            'InstanceID': instance_id,
                            'InstanceType': instance_type,
                            'InstanceCost': float,
                            'NICs': [(NIC ID, EIP ID, ASSOCIATION ID), ...]
                        },
                        ...
                    ]

                Note, for instance rejuvenation, the list is slightly different:
                    [
                            {
                                'InstanceID': instance_id,
                                'InstanceType': instance_type,
                                'InstanceCost': float,
                                'ec2_session': <ec2-session-object>,
                                'ce_session': <ce-session-object>,
                                'NICs': [(NIC ID, EIP ID, ASSOCIATION ID), ...]
                            },
                            ...
                    ]
        """
        start_time = time.time()
        # prices = api.get_cheapest_instance_types_df(initial_client, self.filter, multi_NIC=self.multi_nic)
        function_list = self.loop_create_fleet(initial_client)
        self.print_stdout_and_filename("Create fleet success with details: " + api.pretty_json(function_list), self.print_filename)
        end_time = time.time()
        self.print_stdout_and_filename("Time taken to create fleet: " + str(end_time - start_time), self.print_filename)
        return function_list

    def loop_create_fleet(self, initial_client):
        proxy_count_remaining = self.PROXY_COUNT
        function_list = []
        # ec2_list = []
        # ce_list = []
        # self.print_stdout_and_filename("Original number of rows in prices dataframe: " + str(len(prices.index)), self.print_filename)
        # # df.to_string(header=False, index=False)
        # self.print_stdout_and_filename(prices.to_string(), self.print_filename) # https://stackoverflow.com/a/58070237/13336187
        count = 1
        # prices = prices.reset_index(drop=True) # reset index. https://stackoverflow.com/a/20491748/13336187

        # optimal_cheapest_instance_details = None
        first_iteration = True

        while proxy_count_remaining > 0:
            # index, cheapest_instance = api.get_instance_row_with_supported_architecture(initial_client)
            ### Does Lambda really care architecture? I think only uisng cheapest architecture (ARM) if fine

            # if first_iteration:
            #     max_nics = api.get_max_nics(initial_client, cheapest_instance['InstanceType'])
            #     instances_to_create = math.ceil(self.PROXY_COUNT/max_nics) # this is only used for liveip (i.e., multi-NIC) scenario
            #     optimal_cheapest_instance_details = {"OptimalInstanceCost": cheapest_instance['SpotPrice'], "OptimalInstanceType": cheapest_instance['InstanceType'], "OptimalInstanceZone": cheapest_instance['AvailabilityZone'], "OptimalInstanceMaxNICs": max_nics, "OptimalInstanceCount": instances_to_create}
            #     first_iteration = False

            # prices = prices[index+1:] # if we repeat this loop, it means that we were not able to create enough instances of this type (i.e., index), so we should search from there onwards.
            self.print_stdout_and_filename("Iteration {}".format(count), self.print_filename)
            # df.to_string(header=False, index=False)
            # self.print_stdout_and_filename(prices.to_string(), self.print_filename) # https://stackoverflow.com/a/58070237/13336187
            # cheapest_instance_region = cheapest_instance['AvailabilityZone'][:-1]
            random_region = random.choice(api.US_REGIONS)
            client = api.choose_session(region=random_region)
            function_list_now, proxy_count_remaining = self._create_fleet(client, proxy_count_remaining)

            for funcs in function_list_now:
                funcs['client_session_region'] = random_region
                # funcs['ce_session_region'] = cheapest_instance_region
                # funcs['optimal_cheapest_instance'] = optimal_cheapest_instance_details
            # ec2_list.extend([ec2 for i in range(len(instance_list_now))]) # each instance will have its own ec2 session (in case this is different across instances...)
            # ce_list.extend([ce for i in range(len(instance_list_now))]) # each instance will have its own ce session (in case this is different across instances...)
            function_list.append(function_list_now)
            count += 1

        return function_list

    def rejuvenate(self):
        """Rejuvenates the instances."""
        raise NotImplementedError("Subclasses must implement rejuvenate method")

    def _create_fleet(self, client, proxy_count_remaining):
        """Rejuvenator specific fleet creation."""
        raise NotImplementedError("Subclasses must implement _create_fleet method")

    def handle_reclamation(self):
        """Handles reclamation of resources."""
        pass

    def handle_autoscaling(self):
        """Handles autoscaling of resources."""
        pass


class FunctionRejuvenator(Rejuvenator):
    """Rejuvenator for instances."""

    def __init__(self, initial_region, input_args, filter, tag_prefix, print_filename) -> None:
        """Initializes the InstanceRejuvenator object."""
        super().__init__(initial_region, input_args, filter, tag_prefix, print_filename)

    def rejuvenate(self, quick_test=False):
        """Rejuvenates the instances."""
        """
            Runs in a loop. 

            Parameters:
                rej_period: in seconds
                wait_time_after_create: in seconds
                wait_time_after_nic: in seconds
                    - Time to wait before pinging the instance. This is to allow the instance to be fully instantiated before pinging it.

            Returns: None
        """

        function_lists = []

        self.print_stdout_and_filename("Begin Rejuvenation count: " + str(1), self.print_filename)
        client = api.choose_session(region=self.initial_region)
        # Create fleet (with tag values as indicated above):
        function_list_prev = self.create_fleet(client)
        function_lists.append(function_list_prev)

        # Make sure instance can be sshed/pinged (fail rejuvenation if not):
        # self.print_stdout_and_filename("Waiting for instance NICs to initialize..", self.print_filename)
        # time.sleep(self.wait_time_after_nic)
        client_region = function_list_prev[0][0]['client_session_region']
        client = api.choose_session(region=client_region)
        self.print_stdout_and_filename("Checking if functions are alive..", self.print_filename)
        for index, function_details in enumerate(function_list_prev):
            if function_details[0]['client_session_region'] != client_region:
                client_region = function_details['client_session_region']
                client = api.choose_session(region=client_region)
            # how to check whether Lambda is working or not?
            # failed_ips = api.ping_instances(ec2, instance_details['NICs'], multi_NIC=self.multi_nic)
            # if len(failed_ips) != 0:
            #     self.print_stdout_and_filename("Failed to ssh/ping into instances: " + str(failed_ips), self.print_filename)
            #     assert len(failed_ips) == 0, "Failed to ssh/ping into instances: " + str(failed_ips)
        # self.print_stdout_and_filename("Instances are alive.", self.print_filename)

        # Notify controller:
        if quick_test == True:
            pass
        elif self.initial_proxy_url == "" or self.initial_proxy_url == None:
            self.extract_urls_and_notify_controller([], function_list_prev)
        else:
            # Artifact evaluation purposes:
            assert self.PROXY_COUNT == 1, "Initial proxy IP is provided, so the proxy count must be 1."

            new_urls = []
            print("function_list_prev:")
            print(function_list_prev)
            for function_details in function_list_prev[0]:
                # for url in function_details['FunctionURL']:
                #     new_urls.append(url[-1])
                new_urls.append(function_details['FunctionURL'])
            if self.initial_proxy_url in new_urls:
                new_urls.remove(self.initial_proxy_url)
            print(f"old url:{self.initial_proxy_url}")
            print(f"new url:{new_urls}")
            self.notify_controller([self.initial_proxy_url], [self.initial_proxy_arn], new_urls)
        
        # Sleep for rej_period:
        self.print_stdout_and_filename("Sleeping for {} seconds until next rejuvenation period..".format(self.REJUVENATION_PERIOD), self.print_filename)
        time.sleep(self.REJUVENATION_PERIOD)
        # Continue with rejuvenation:
        rejuvenation_index = 2
        while True and quick_test == False:
            # refresh_credentials() # assume usage of permanent credentials for now
            # print("Begin Rejuvenation count: ", rejuvenation_index)
            self.print_stdout_and_filename("Begin Rejuvenation count: " + str(rejuvenation_index), self.print_filename)

            # Create fleet (with tag values as indicated above):
            client = api.choose_session(region=self.initial_region)
            function_list = self.create_fleet(client)

            # Make sure instance can be sshed/pinged (fail rejuvenation if not):
            # self.print_stdout_and_filename("Waiting for instance NICs to initialize..", self.print_filename)
            # time.sleep(self.wait_time_after_nic)
            self.print_stdout_and_filename("Checking if functions are alive..", self.print_filename)
            client_region = function_list[0][0]['client_session_region']
            client = api.choose_session(region=client_region)
            for index, function_details in enumerate(function_list):
                if function_details[0]['client_session_region'] != client_region:
                    client_region = function_details[0]['client_session_region']
                    client = api.choose_session(region=client_region)
            #     failed_ips = api.ping_instances(ec2, instance_details['NICs'], multi_NIC=False)
            #     if len(failed_ips) != 0:
            #         self.print_stdout_and_filename("Failed to ssh/ping into instances: " + str(failed_ips), self.print_filename)
            #         assert len(failed_ips) == 0, "Failed to ssh/ping into instances: " + str(failed_ips)
            # self.print_stdout_and_filename("Instances are alive.", self.print_filename)

            # Notify controller:
            self.extract_urls_and_notify_controller(function_list_prev, function_list)

            # Terminate fleet:
            # Chunk list into groups of 100: Maybe work on this next time..
            # chunk_terminate_instances(instance_list_prev, 100)
            # def chunk_terminate_instances(instance_list, chunk_size):
            #     for index, chunk in enumerate(list(chunks(instance_list, chunk_size))):
            #         instance_ids = [instance_details['InstanceID'] for instance_details in chunk]
            #         for ec2 in ec2_list:
            #             api.terminate_instances(ec2, instance_ids)

            # Sleep for rej_period:
            self.print_stdout_and_filename("Sleeping for {} seconds until next rejuvenation period..".format(self.REJUVENATION_PERIOD), self.print_filename)
            time.sleep(self.REJUVENATION_PERIOD)

            self.print_stdout_and_filename("Terminating functions from previous rejuvenation period..", self.print_filename)
            for index, function_details in enumerate(function_list_prev):
                if function_details[0]['client_session_region'] != client_region:
                    client_region = function_details[0]['client_session_region']
                    client = api.choose_session(region=client_region)
                function = function_details[0]['FunctionArn']
                api.delete_functions(client, function)
            self.print_stdout_and_filename("Functions terminated from previous rejuvenation period..", self.print_filename)

            # To be terminated in next rejuvenation:
            function_list_prev = function_list
            function_lists.append(function_list_prev)
            
            # # Sleep for rej_period:
            # self.print_stdout_and_filename("Sleeping for {} seconds until next rejuvenation period..".format(self.REJUVENATION_PERIOD), self.print_filename)
            time.sleep(self.REJUVENATION_PERIOD/2)

            # Print new details:
            # print("Concluded Rejuvenation count: ", rejuvenation_index)
            self.print_stdout_and_filename("Concluded Rejuvenation count: " + str(rejuvenation_index), self.print_filename)
            # print("New instance details: ", pretty_json(instance_list))
            self.print_stdout_and_filename("New instance details: " + api.pretty_json(function_list), self.print_filename)
            rejuvenation_index += 1
        
        # Terminate remaining instances:
        # refresh_credentials()
        self.print_stdout_and_filename("Terminating functions from previous rejuvenation period..", self.print_filename)
        client_region = function_list_prev[0][0]['client_session_region']
        client = api.choose_session(region=client_region)
        for index, function_details in enumerate(function_list_prev):
            instance = function_details[0]['FunctionArn']
            if function_details[0]['client_session_region'] != client_region:
                client_region = function_details[0]['client_session_region']
                client= api.choose_session(region=client_region)
            api.delete_functions(client, instance)
        self.print_stdout_and_filename("Functions terminated from previous rejuvenation period..", self.print_filename)
        
        # Get total cost:
        # total_cost, optimal_total_cost, total_monthly_cost, optimal_monthly_cost = calculate_cost(instance_lists, rej_period, exp_duration, multi_NIC=False, rej_count=rejuvenation_index-1)
        # print_stdout_and_filename("Total cost of this instance rejuvenation experiment: {}. Optimal total cost (single-NIC) is: {}".format(total_cost, optimal_total_cost), print_filename)
        # print_stdout_and_filename("Total monthly cost of this instance rejuvenation experiment: {}. Optimal total monthly cost (single-NIC) is: {}".format(total_monthly_cost, optimal_monthly_cost), print_filename)
        # # print("Total cost of this instance rejuvenation experiment: {}".format())

        return 

    def _create_fleet(self, client, proxy_count_remaining):
        global FLEETIDX 
        tag = 'fleet-'+str(FLEETIDX)
    # def create_fleet_instance_rejuvenation(ec2, cheapest_instance, proxy_count, proxy_impl, tag_prefix, wait_time_after_create=15, print_filename="data/output-general.txt"):
        """
            Creates fleet combinations. 

            Parameters:
                - cheapest_instance: row of the cheapest instance type (from get_cheapest_instance_types_df)
                - proxy_impl: "snowflake" | "wireguard" | "baseline"
                - tag_prefix: "instance-expX" 
                - wait_time_after_create: in seconds
        """
        # Get the cheapest instance now:
        # instance_type_cost = cheapest_instance['SpotPrice']
        # instance_type = cheapest_instance['InstanceType']
        # zone = cheapest_instance['AvailabilityZone']
        zone = random.choice(api.US_REGIONS)
        # region = zone[:-1] # e.g., us-east-1a -> us-east-1

        # Get suitable launch template based on the region associated with the zone:
        # launch_template = self.launch_templates[0] # only 1 launch template for now..

        # Create the initial fleet with multiple NICs (with tag values as indicated above)
        for _ in range(proxy_count_remaining):
            response = api.create_function(client, zone, image, role, tag)
        time.sleep(self.wait_time_after_create) # wait awhile for fleet to be created
        # make sure that the required instances have been acquired: 
        # print(response['FleetId'])
        all_instance_details = api.get_specific_functions_with_fleet_id_tag(client, zone, tag, "raw") 
        if len(all_instance_details) != proxy_count_remaining: 
            warnings.warn("Not enough functions were created: only created " + str(len(all_instance_details)) + " functions, but " + str(proxy_count_remaining) + " were required.")
            self.print_stdout_and_filename("Not enough functions were created: only created " + str(len(all_instance_details)) + " functions, but " + str(proxy_count_remaining) + " were required.", self.print_filename)
        proxy_count_remaining = proxy_count_remaining - len(all_instance_details)

        # print("Created {} instances of type {}, and hourly cost {}. Remaining instances to create: {}".format(len(all_instance_details), instance_type, instance_type_cost, proxy_count_remaining))
        self.print_stdout_and_filename("Created {} functions. Remaining functions to create: {}".format(len(all_instance_details), proxy_count_remaining), self.print_filename)

        instance_list = []

        for index, original_instance_details in enumerate(all_instance_details):
            func = original_instance_details['FunctionArn']
            function_url = client.list_function_url_configs(FunctionName=func)
            
            # Tag created instance:
            # instance_tag = self.tag_prefix + "-instance{}".format(str(index))
            # api.assign_name_tags(ec2, instance, instance_tag) # TODO: removed for now pending increase limit..
            
            instance_details = {'FunctionArn': func, 'FunctionURL': function_url['FunctionUrlConfigs'][0]['FunctionUrl'], 'FunctionType': original_instance_details['Architectures']}
            # Get original NIC attached to the instance:
            # original_nic = original_instance_details['NetworkInterfaces'][0]['NetworkInterfaceId']
            # original_pub_ip = original_instance_details['PublicIpAddress']
            # # assert len(original_instance_details['NetworkInterfaces']) == 1, "Expected only 1 NIC, but got " + str(len(original_instance_details['NetworkInterfaces']))
            # # _ , original_nic = api.get_specific_instances_attached_components(ec2, instance)

            # # Tag the original NIC:
            # instance_details['NICs'] = [(original_nic, original_pub_ip)]
            # nic_tag = instance_tag + "-nic{}".format(str(1))
            # api.assign_name_tags(ec2, original_nic, nic_tag) # TODO: removed for now pending increase limit..

            instance_list.append(instance_details) 

        FLEETIDX += 1
        print(tag)
        return instance_list, proxy_count_remaining

    # def handle_reclamation(self):
    #     """Handles reclamation of resources."""
    #     super().handle_reclamation()

    # def handle_autoscaling(self):
    #     """Handles autoscaling of resources."""
    #     super().handle_autoscaling()