import instance_manager.api as api, json
initial_client, = api.choose_session(is_UM_AWS=True, region='us-east-1')
# Excluded instances:
excluded_instances = api.get_excluded_terminate_instances()
instances_terminated = api.nuke_all_functions(initial_client, excluded_instances)
print("NUKED EVERYTING. Total instances terminated: " + str(len(instances_terminated)))