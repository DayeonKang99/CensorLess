# Artifact Appendix 

Paper title: **CensorLess: Cost-Efficient Censorship Circumvention Through Serverless Cloud Functions**

Requested Badge(s):
  - [x] **Available**
  - [x] **Functional**
  - [ ] **Reproduced**


## Description 

Related works
- SpotProxy: Rediscovering the Cloud for Censorship Circumvention, Kon et al. 2024
```
@inproceedings{kon2024spotproxy,
  title={$\{$SpotProxy$\}$: Rediscovering the Cloud for Censorship Circumvention},
  author={Kon, Patrick Tser Jern and Kamali, Sina and Pei, Jinyu and Barradas, Diogo and Chen, Ang and Sherr, Micah and Yung, Moti},
  booktitle={33rd USENIX Security Symposium (USENIX Security 24)},
  pages={2653--2670},
  year={2024}
}
```
- https://github.com/TooTallNate/proxy-agents/tree/main/packages/proxy 

We borrowed the bridge migration mechanism (`/censorless-vanilla/function-refresher` and `/censorless-vanilla/controller`) and censor simulation code that SpotProxy implemented. 
For the function-refresher, we modified the code to work in the serverless environment. 
We implemented a local proxy server upon the existing proxy code from `proxy-agent`. 


### Security/Privacy Issues and Ethical Concerns 

The vanilla CensorLess implementation reassembles HTTPS packets at serverless bridges, creating a trust relationship between users and bridge operators who have technical access to user traffic. We explicitly acknowledge this limitation and provide a privacy-preserving mode with encrypted channels at increased cost. As a consequence of CensorLess being available to the public, careful evaluation of the operator's trustworthiness by users is required. We recommend operating serverless bridges for individual purposes or by trustworthy organizations with strict no-logging policies. 

Due to the HTTPS decapsulation in the vanilla mode, the local proxy server intercepts the user's request and translates it to an HTTPS GET/POST request. However, it only happens in the user's local machine, and a security update is required. 


## Basic Requirements 

For both sections below, if you are giving reviewers remote access to special
hardware (e.g., Intel SGX v2.0) or proprietary software (e.g., Matlab R2025a)
for the purpose of the artifact evaluation, do not provide these instructions
here but rather in the corresponding submission field on HotCRP.

### Hardware Requirements 

- CensorLess vanilla mode
   - Can run on a laptop (No special hardware requirements)
- CensorLess private mode
   - 

Replace this with the following:

1. A list of the _minimal hardware requirements_ to execute your artifact. If no
   specific hardware is needed, then state "Can run on a laptop (No special
   hardware requirements)". You may state how a researcher could gain access to
   that hardware, e.g., by buying, renting, or even emulating it.
2. When applying for the "Reproduced" badge, list _the specifications of the
   hardware_ on which the experiments reported in the paper were performed. This
   is especially relevant in cases were results might be influenced by the
   hardware used (e.g., latency, bandwidth, throughput experiments, etc.).

### Software Requirements 

- CensorLess vanilla mode
   - OS: Ubuntu 22.04
   - Container: any version of Docker
   - Packages: `pnpm`, AWS CLI, [conda](https://www.anaconda.com/docs/getting-started/miniconda/main#quick-command-line-install)
- CensorLess private mode
   - 

Replace this with the software required to run your artifact and its versions,
as follows.

1. List the OS you used to run your artifact, along with its version (e.g.,
   Ubuntu 22.04). If your artifact can only run on a specific OS or a specific
   OS version, list it and explain why here. In general, your artifact reviewers
   will probably have access to a machine with a different OS or different OS
   version than yours; they should still be able to run appropriately packaged
   artifacts.
2. List the OS packages that your artifact requires, along with their versions.
3. Artifact packaging: If you use a container runtime (e.g., Docker) to run the
   artifact, list the container runtime and its version (e.g., Docker 23.0.3).
   If you use VMs, list the hypervisor (e.g., VirtualBox) to run the artifact.
4. List the programming language compiler or interpreter you used to run your
   artifact (e.g., Python 3.13.7). Your Docker image or VM image should have
   this version of the programming languages installed already. Your Dockerfile
   should start from a base image with this programming language version.
5. List packages that your artifact depends on, along with their versions. For
   example, Python-based privacy-preserving machine learning artifacts typically
   require `numpy`, `scipy`, etc. You may point to a file in your artifact with
   this list, such as a `requirements.txt` file. If you rely on proprietary
   software (e.g. Matlab R2025a), list this here and consider providing access
   to reviewers through HotCRP.
6. List any Machine Learning Models required to run your artifact, along with
   their versions. If your model is hosted on a different repository, such as on
   Zenodo, then your artifact should download it automatically (same for
   datasets). If a required ML model is _not_ in your artifact, provide a dummy
   model to demonstrate the functionality of the rest of your artifact.
7. List any datasets required to run your artifact. If any required dataset is
   not in your artifact, you should provide a synthetic dataset that showcases
   the expected data format.

### Estimated Time and Storage Consumption 

Assuming the CensorLess is evaluated through the provided codes in eval, `censored_domain` testing will take approximately 3 hours for each version, and `pcap_capture_use_cases` will take less than 1 hour for each version.
It does not consume large disk space (less than 1MB).


Replace the following with estimated values for:

- The overall human and compute times required to run the artifact.
- The overall disk space consumed by the artifact.

This helps reviewers schedule the evaluation in their time plan and others in
general to see if everything is running as intended. This should also be
specified at a finer granularity for each experiment (see below).

## Environment 

In the following, describe how to access your artifact and all related and
necessary data and software components. Afterward, describe how to set up
everything and how to verify that everything is set up correctly.

### Accessibility 

Code is available here: <https://github.com/DayeonKang99/CensorLess/tree/main>

### Set up the environment 

```bash
git clone <this repo>
```

- CensorLess vanilla mode
   - serverless-bridge: go to <https://github.com/DayeonKang99/CensorLess/tree/main/censorless-vanilla/serverless-bridge/README.md> 
   - local-proxy: 
   - controller: 
      1. 
      ```bash
      sudo docker build -t controller-image -f Dockerfile .
      sudo docker run -it -p 8000:8000 --cap-add=NET_ADMIN \
      -e AWS_ACCESS_KEY_ID=<your-access-key> \
      -e AWS_SECRET_ACCESS_KEY=<your-secret-key> \
      -e AWS_DEFAULT_REGION=<aws-region> \
      --name=controller controller-image
      ```
      2. go to <http://0.0.0.0:8000/admin> (username: `admintest` and password: `123`)
      3. Create the following objects:
         - Create a new Proxy object: set the serverless bridge URL, latitude, and longitude.
         - Create a new Client object: set the IP of client, latitude, and longitude.
         - Create a new Assignment object: select the newly created proxy and client object as appropriate.
   - function-refresher: 
      1. fill out "initial_proxy_url" and "initial_proxy_arn" of the created serverless bridge in [this set up](https://github.com/DayeonKang99/CensorLess/tree/main/censorless-vanilla/serverless-bridge/README.md)
      2. ```bash
         cd censorless-vanilla/function-refresher/
         conda env create -f environment.yml
         conda activate function-refresher
         python3 api.py input-args.json
         ```
- CensorLess private mode
   - 

As a brief test, when you issue the `curl` command, you can see that the local proxy fetches the serverless bridge URL periodically, and your command returned the response. 
```bash
curl -x http://localhost:8080 [url]
```

Replace the following by a description of how one should set up the environment
for your artifact, including downloading and installing dependencies and the
installation of the artifact itself (i.e., from the very first download or clone
command one should perform). Be as specific as possible here. If possible, use
code segments to simplify the workflow, e.g.,

```bash
git clone git@github.com:PoPETS-AEC/example-docker-python-pip.git
docker build -t example-docker-python-pip:main .
```

Describe the expected results where it makes sense to do so.

### Testing the Environment 

- CensorLess vanilla mode
   - without migration: set up `serverless-bridge` and `local-proxy` only
   - with migration: set up `controller` and `function-refresher` additionally
- CensorLess private mode


```bash
cd eval/censored_domains
./health_check.sh 50_well_known_blocked_domains.txt <proxy> <outputfile>
```
Use `./health_check.sh` to test censorless vanilla, and use `./health_check_v2.sh` to test censorless private mode.



Replace the following by a description of the basic functionality tests to check
if the environment is set up correctly. These tests could be unit tests,
training an ML model on very low training data, etc. If these tests succeed, all
required software should be functioning correctly. Use code segments to simplify
the workflow, e.g.,

Launch the Docker container, attach the current working directory (i.e., run
from the root of the cloned git repository) as a volume, set the context to be
that volume, and provide an interactive bash terminal:

```bash
docker run --rm -it -v ${PWD}:/workspaces/example-docker-python-pip \
    -w /workspaces/example-docker-python-pip \
    --entrypoint bash example-docker-python-pip:main
```

Then within the Docker container, run:

```bash
./test.sh
```

Include the expected output.

## Artifact Evaluation 

This section should include all the steps required to evaluate your artifact's
functionality and validate your paper's key results and claims. Therefore,
highlight your paper's main results and claims in the first subsection. And
describe the experiments that support your claims in the subsection after that.

### Main Results and Claims

- Figure 3 (throughput graphs in three different use cases) and Figure 9 (waterfall graphs in three different use cases)
- Tables 1 and 3 (500 website access results)
- Figures 8 and 12 (censor simulation results)

List all your paper's results and claims that are supported by your submitted
artifacts.

#### Main Result 1: User interaction test in three different use cases

Figure 3: The independent variable is time (seconds) and dependent variable is throughput (Kbps). It shows that CensorLess vanilla mode, regardless of migration, takes a longer time compared to without proxy and private mode in terms of the total execution time, even though the private mode requires more time for receiving the first response due to the secure connection setup process.

Figure 9: The independent variable is time since first row (seconds) and y-axis presents he destination where the request goes. The vanilla with migration seamlessly handles the client requests even though the bridge migration happened, and private mode requires more time to load the webpage.

#### Main Result 2: Website access in censored regions

Our paper claims that both the CensorLess vanilla mode and the private mode receive responses to every request sent to blocked domains, except for the blocking by website policy, bypassing the censorship in Nanjing, China. This experiment was conducted in limited settings (i.e., tested in the cloud network), so the experiment result may be different in other settings. 

#### Main Result 3: Censor simulation

We experimented with a censor simulation framework by measuring the connected user ratio and nonblocked proxy ratio as time goes on, setting the bridge migration period the same as the blocking period and twice the blocking period. CensorLess demonstrates even more stable and higher connectivity across both client and proxy dimensions, even when using less frequent refreshing.

### Experiments
List each experiment to execute to reproduce your results. Describe:
 - How to execute it in detailed steps.
 - What the expected result is.
 - How long it takes to execute in human and compute times (approximately).
 - How much space it consumes on disk (approximately) (omit if <10GB).
 - Which claim and results does it support, and how.

#### Experiment 1: User interaction test in three different use cases

- Time: 10 human-minutes + 6 compute-hours

```bash
cd eval/censored_domains
./health_check.sh blocked_domains.txt <proxy> <outputfile>
```
Use `./health_check.sh` to test censorless vanilla, and use `./health_check_v2.sh` to test censorless private mode. 
Since CensorLess vanilla only accepts HTTP at the client-side (outgoing requests are transferred to HTTPS) and CensorLess private allows HTTPS requests directly from the client, the testing code is different.

Expected output: 
health check results in CSV files

As the censored region, China, currently adopts the censorship strategy that prevents users from receiving content from these websites over time, the TIMEOUT results from a CSV file indicate that the request is censored. Any returned health check results (e.g., 200 (OK), 301 (Moved Permanently), 302 (Moved Temporarily)) show that the client successfully received a response from blocked domains. 


- Time: replace with estimate in human-minutes/hours + compute-minutes/hours.
- Storage: replace with estimate for disk space used (omit if <10GB).

Provide a short explanation of the experiment and expected results. Describe
thoroughly the steps to perform the experiment and to collect and organize the
results as expected from your paper (see example below). Use code segments to
simplify the workflow, as follows.

```bash
python3 experiment_1.py
```

#### Experiment 2: Website access in censored regions

- Time: 10 human-minutes + 1 compute-hours
 
```bash
cd eval
python pcap_capture_use_cases.py --use-case 1 --interface <interface#> --output-dir <directory> --proxy <proxy>
```

Expected output: 
PCAP files and event logs

We conducted this experiment in four different settings: without proxy, CensorLess vanilla, vanilla with migration, and CensorLess private. From the output PCAP files, we plotted the throughput and waterfall graph. As the throughput and content load time vary every time executing, the result might be different from the paper. Our experiment results are visualized in Figures 3 and 9.


#### Experiment 3: Censor simulation

- Time: 3 human-minutes + 10 compute-minutes

```bash
cd censorless-vanilla/controller
sudo docker build -t simulation-image -f SimulationDockerfile .
sudo docker run -it --name=simulation simulation-image
```

The results can be inspected as follows:
```bash
sudo docker ps --all # Get the CONTAINER ID of the docker container that you ran earlier 
sudo docker commit <container-ID> test-commit # Create a new commit based on the current state of the container
sudo docker run -it test-commit /bin/bash # Access the docker container
ls results # Retrieve the required simulation output file within this folder
```
(The censor simulation process is from the SpotProxy artifact.)

Expected output: 
CSV file in `results` directory

Parameters that we used for Figures 8 and 12 can be found in the paper. 

## Limitations 

Describe which steps, experiments, results, graphs, tables, etc. are _not
reproducible_ with the provided artifact. Explain why this is not
included/possible and argue why the artifact should _still_ be evaluated for the
respective badges.

## Notes on Reusability 

First, this section might not apply to your artifacts. Describe how your
artifact can be used beyond your research paper, e.g., as a general framework.
The overall goal of artifact evaluation is not only to reproduce and verify your
research but also to help other researchers to re-use and extend your artifacts.
Discuss how your artifacts can be adapted to other settings, e.g., more input
dimensions, other datasets, and other behavior, through replacing individual
modules and functionality or running more iterations of a specific module.
