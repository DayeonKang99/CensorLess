import os
import time
import subprocess
import argparse
from datetime import datetime
from selenium import webdriver
from selenium.webdriver.firefox.options import Options as FirefoxOptions
from selenium.webdriver.chrome.options import Options as ChromeOptions
from selenium.webdriver.common.by import By
from selenium.webdriver.common.action_chains import ActionChains
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC
from selenium.common.exceptions import TimeoutException, NoSuchElementException
import signal
import sys
import json


class PcapCaptureTester:
    def __init__(self, output_dir="pcap_captures", interface="any", proxy=None, proxy_user=None, proxy_pass=None):
        """Initialize the PCAP capture tester.
        
        Args:
            output_dir (str): Directory where PCAP files will be saved
            interface (str): Network interface to capture on (default: 'any')
            proxy (str): Proxy server to use (format: protocol://host:port)
            proxy_user (str): Username for proxy authentication
            proxy_pass (str): Password for proxy authentication
        """
        self.output_dir = output_dir
        self.interface = interface
        self.proxy = proxy
        self.proxy_user = proxy_user
        self.proxy_pass = proxy_pass
        self.tcpdump_process = None
        
        # Create output directory if it doesn't exist
        if not os.path.exists(self.output_dir):
            os.makedirs(self.output_dir)
            print(f"Created output directory: {self.output_dir}")
    
    def start_packet_capture(self, pcap_filename):
        """Start packet capture using tcpdump (Linux/Mac) or tshark (Windows).
        
        Args:
            pcap_filename (str): Name of the PCAP file to save
            
        Returns:
            subprocess.Popen: The packet capture process
        """
        pcap_path = os.path.join(self.output_dir, pcap_filename)
        
        # Determine which tool to use based on platform
        if sys.platform == 'win32':
            # Windows: Use tshark (part of Wireshark)
            # Default Wireshark installation path
            tshark_paths = [
                r"C:\Program Files\Wireshark\tshark.exe",
                r"C:\Program Files (x86)\Wireshark\tshark.exe",
                "tshark"  # If in PATH
            ]
            
            tshark_exe = None
            for path in tshark_paths:
                if os.path.exists(path) or path == "tshark":
                    tshark_exe = path
                    break
            
            if not tshark_exe:
                print("ERROR: tshark not found. Please install Wireshark from https://www.wireshark.org/")
                print("Make sure to install with command-line tools (tshark)")
                return None
            
            # tshark command for Windows
            cmd = [
                tshark_exe,
                "-i", self.interface,
                "-w", pcap_path,
                "-q"  # Quiet mode
            ]
            
            print(f"Starting packet capture (Windows): {pcap_path}")
            print(f"Command: {' '.join(cmd)}")
            print("Note: You may need to run as Administrator for packet capture")
            
        else:
            # Linux/Mac: Use tcpdump (requires sudo)
            cmd = [
                "sudo", "tcpdump",
                "-i", self.interface,
                "-w", pcap_path,
                "-s", "0",  # Capture full packets
                "-U"  # Write packets immediately
            ]
            
            print(f"Starting packet capture (Linux/Mac): {pcap_path}")
            print(f"Command: {' '.join(cmd)}")
        
        try:
            process = subprocess.Popen(
                cmd,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if sys.platform == 'win32' else 0
            )
            time.sleep(2)  # Give the tool time to start
            
            # Check if process started successfully
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                print(f"Failed to start packet capture:")
                print(f"stdout: {stdout.decode() if stdout else ''}")
                print(f"stderr: {stderr.decode() if stderr else ''}")
                return None
            
            print("Packet capture started successfully")
            return process
            
        except FileNotFoundError:
            if sys.platform == 'win32':
                print("ERROR: tshark not found. Please install Wireshark from https://www.wireshark.org/")
                print("Download page: https://www.wireshark.org/download.html")
            else:
                print("ERROR: tcpdump not found. Please install tcpdump:")
                print("  Ubuntu/Debian: sudo apt-get install tcpdump")
                print("  macOS: brew install tcpdump")
            return None
        except Exception as e:
            print(f"Error starting packet capture: {e}")
            if sys.platform == 'win32':
                print("Note: You may need to run this script as Administrator")
            else:
                print("Note: tcpdump requires sudo privileges")
            return None
    
    def stop_packet_capture(self, process):
        """Stop the tcpdump/tshark process.
        
        Args:
            process (subprocess.Popen): The packet capture process to stop
        """
        if process:
            print("Stopping packet capture...")
            
            # Windows doesn't support SIGINT, use terminate() instead
            try:
                if sys.platform == 'win32':
                    process.terminate()
                else:
                    process.send_signal(signal.SIGINT)
                
                # Wait for process to terminate
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    print("Process didn't terminate gracefully, forcing kill...")
                    process.kill()
                    process.wait()
                    
            except Exception as e:
                print(f"Error stopping process: {e}")
                try:
                    process.kill()
                except:
                    pass
            
            print("Packet capture stopped")
    
    def setup_browser(self, browser_type="firefox"):
        """Set up browser with proxy configuration.
        
        Args:
            browser_type (str): Browser to use ('firefox' or 'chrome')
            
        Returns:
            webdriver: Configured browser driver
        """
        if browser_type == "firefox":
            options = FirefoxOptions()
            
            # Configure proxy if specified
            if self.proxy:
                parsed_proxy = self.proxy.split('://')
                proxy_protocol = parsed_proxy[0] if len(parsed_proxy) > 1 else 'http'
                proxy_address = parsed_proxy[-1]
                
                print(f"Using proxy: {self.proxy}")
                
                if proxy_protocol.startswith('socks'):
                    # SOCKS proxy configuration
                    socks_version = 5 if proxy_protocol in ['socks5', 'socks5h'] else 4
                    host, port = proxy_address.split(':')
                    options.set_preference("network.proxy.type", 1)
                    options.set_preference("network.proxy.socks", host)
                    options.set_preference("network.proxy.socks_port", int(port))
                    options.set_preference("network.proxy.socks_version", socks_version)
                    options.set_preference("network.proxy.socks_remote_dns", proxy_protocol.endswith('h'))
                else:
                    # HTTP/HTTPS proxy configuration
                    options.set_preference("network.proxy.type", 1)
                    if proxy_protocol == 'http':
                        host, port = proxy_address.split(':')
                        options.set_preference("network.proxy.http", host)
                        options.set_preference("network.proxy.http_port", int(port))
                        options.set_preference("network.proxy.ssl", host)
                        options.set_preference("network.proxy.ssl_port", int(port))
                
                # Add proxy authentication if provided
                if self.proxy_user and self.proxy_pass:
                    options.set_preference("network.proxy.share_proxy_settings", True)
                    options.set_preference("signon.autologin.proxy", True)
            
            driver = webdriver.Firefox(options=options)
            
        elif browser_type == "chrome":
            options = ChromeOptions()
            
            # Configure proxy if specified
            if self.proxy:
                options.add_argument(f'--proxy-server={self.proxy}')
            
            driver = webdriver.Chrome(options=options)
        
        else:
            raise ValueError(f"Unsupported browser type: {browser_type}")
        
        return driver
    
    def use_case_1_cnn_browsing(self, repetitions=5, browser_type="firefox"):
        """Use Case 1: Browse CNN.com, scroll, and click articles.
        
        Args:
            repetitions (int): Number of times to repeat scrolling and clicking
            browser_type (str): Browser to use
        """
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        pcap_filename = f"usecase1_cnn_browsing_{timestamp}.pcap"
        log_filename = f"usecase1_cnn_browsing_{timestamp}.log"
        
        # Initialize event log
        event_log = []
        
        def log_event(event_type, description=""):
            """Helper function to log events with timestamps"""
            event_timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
            event_entry = {
                "timestamp": event_timestamp,
                "epoch_time": time.time(),
                "event_type": event_type,
                "description": description
            }
            event_log.append(event_entry)
            print(f"[{event_timestamp}] {event_type}: {description}")
        
        print("\n" + "="*60)
        print("USE CASE 1: CNN.com Browsing with Scrolling and Clicking")
        print("="*60)
        
        log_event("START", "Use Case 1 started")
        
        # Start packet capture
        log_event("CAPTURE_START", "Starting packet capture")
        capture_process = self.start_packet_capture(pcap_filename)
        
        if not capture_process:
            print("Failed to start packet capture. Aborting use case.")
            log_event("ERROR", "Failed to start packet capture")
            return
        
        driver = None
        try:
            log_event("BROWSER_INIT", f"Initializing {browser_type} browser")
            driver = self.setup_browser(browser_type)
            driver.maximize_window()
            
            # Set page load timeout to avoid hanging
            driver.set_page_load_timeout(30)
            
            # Navigate to CNN
            print("Navigating to CNN.com...")
            log_event("PAGE_LOAD_START", "Loading CNN.com")
            try:
                driver.get("http://www.cnn.com")
                log_event("PAGE_LOAD_END", "CNN.com loaded successfully")
            except TimeoutException:
                print("Initial page load timed out, but continuing...")
                log_event("PAGE_LOAD_TIMEOUT", "CNN.com load timed out")
            
            time.sleep(5)  # Wait for page to load
            
            # Handle terms agreement popup if it appears
            try:
                print("Checking for terms agreement popup...")
                accept_button = WebDriverWait(driver, 5).until(
                    EC.element_to_be_clickable((By.XPATH, 
                        "//button[contains(text(), 'Accept') or contains(text(), 'Agree') or contains(@class, 'accept')]"))
                )
                print("Found terms agreement button, clicking...")
                log_event("POPUP_INTERACTION", "Accepting terms agreement")
                accept_button.click()
                time.sleep(2)
                print("Terms accepted")
            except TimeoutException:
                print("No terms agreement popup found (or already accepted)")
            except Exception as e:
                print(f"Could not handle terms popup: {e}")
                print("Continuing anyway...")
            
            # Alternative: Try to close any modal/overlay
            try:
                close_buttons = driver.find_elements(By.CSS_SELECTOR, 
                    "button.close, button[aria-label='Close'], .modal-close, .overlay-close")
                for btn in close_buttons:
                    try:
                        if btn.is_displayed():
                            btn.click()
                            time.sleep(1)
                    except:
                        pass
            except:
                pass
            
            for i in range(repetitions):
                print(f"\n--- Iteration {i+1}/{repetitions} ---")
                log_event("ITERATION_START", f"Starting iteration {i+1}/{repetitions}")
                
                # Scroll down the page
                print("Scrolling down...")
                try:
                    log_event("SCROLL", "Scrolling to 1/3 of page")
                    driver.execute_script("window.scrollTo(0, document.body.scrollHeight/3);")
                    time.sleep(2)
                    
                    log_event("SCROLL", "Scrolling to 2/3 of page")
                    driver.execute_script("window.scrollTo(0, 2*document.body.scrollHeight/3);")
                    time.sleep(2)
                    
                    log_event("SCROLL", "Scrolling to bottom of page")
                    driver.execute_script("window.scrollTo(0, document.body.scrollHeight);")
                    time.sleep(3)
                except Exception as e:
                    print(f"Error scrolling: {e}")
                    log_event("ERROR", f"Scrolling error: {e}")
                
                # Try to find and click an article
                try:
                    print("Looking for article links...")
                    log_event("SCROLL", "Scrolling back to top")
                    # Scroll back to top to find fresh articles
                    driver.execute_script("window.scrollTo(0, 0);")
                    time.sleep(2)
                    
                    log_event("ARTICLE_SEARCH", "Searching for article links")
                    # Find article links (CNN uses various selectors)
                    article_links = driver.find_elements(By.CSS_SELECTOR, 
                        "a[data-link-type='article'], h3 a, .container__link, article a")
                    
                    # Filter for valid links with href
                    valid_links = [link for link in article_links 
                                if link.get_attribute('href') and 
                                'cnn.com' in link.get_attribute('href')]
                    
                    log_event("ARTICLE_SEARCH", f"Found {len(valid_links)} valid article links")
                    
                    if valid_links and len(valid_links) > 0:
                        # Use modulo to cycle through available links
                        link_index = i % len(valid_links)
                        target_link = valid_links[link_index]
                        article_url = target_link.get_attribute('href')
                        
                        print(f"Clicking on article {link_index + 1}: {article_url[:80]}...")
                        log_event("ARTICLE_CLICK", f"Clicking article: {article_url}")
                        
                        try:
                            # Strategy 1: Scroll element into view and wait
                            driver.execute_script("arguments[0].scrollIntoView({block: 'center'});", target_link)
                            time.sleep(1)
                            
                            # Strategy 2: Try to remove/hide overlapping ads
                            try:
                                driver.execute_script("""
                                    // Hide common ad elements that might be in the way
                                    var ads = document.querySelectorAll('.ad-slot-header, .ad-slot, .advertisement, [class*="ad-"], [id*="ad-"]');
                                    ads.forEach(function(ad) {
                                        if (ad) ad.style.display = 'none';
                                    });
                                    
                                    // Hide sticky headers that might obstruct
                                    var headers = document.querySelectorAll('header[class*="sticky"], [class*="sticky-header"]');
                                    headers.forEach(function(header) {
                                        if (header) header.style.position = 'static';
                                    });
                                """)
                                time.sleep(0.5)
                            except:
                                pass
                            
                            # Strategy 3: Try normal click first
                            try:
                                target_link.click()
                                print("Clicked successfully with normal click")
                                log_event("ARTICLE_CLICK_SUCCESS", "Normal click succeeded")
                            except Exception as click_error:
                                print(f"Normal click failed: {click_error}")
                                
                                # Strategy 4: Try JavaScript click as fallback
                                try:
                                    print("Trying JavaScript click...")
                                    driver.execute_script("arguments[0].click();", target_link)
                                    print("Clicked successfully with JavaScript")
                                    log_event("ARTICLE_CLICK_SUCCESS", "JavaScript click succeeded")
                                except Exception as js_error:
                                    print(f"JavaScript click failed: {js_error}")
                                    
                                    # Strategy 5: Navigate directly to URL
                                    print(f"Navigating directly to: {article_url}")
                                    log_event("ARTICLE_NAVIGATE", f"Direct navigation to {article_url}")
                                    try:
                                        driver.get(article_url)
                                    except TimeoutException:
                                        print("Direct navigation timed out, but continuing...")
                                        log_event("PAGE_LOAD_TIMEOUT", "Article load timed out")
                            
                            # Wait for new page with timeout
                            time.sleep(5)
                            
                            print(f"Article loaded: {driver.current_url[:80]}...")
                            log_event("ARTICLE_LOAD_COMPLETE", f"Article page: {driver.current_url}")
                            log_event("READ_ARTICLE_START", "Reading article")
                            
                            # Scroll the article
                            print("Scrolling article...")
                            try:
                                log_event("SCROLL", "Scrolling article to 50%")
                                driver.execute_script("window.scrollTo(0, document.body.scrollHeight/2);")
                                time.sleep(2)
                                log_event("SCROLL", "Scrolling article to bottom")
                                driver.execute_script("window.scrollTo(0, document.body.scrollHeight);")
                                time.sleep(2)
                                log_event("READ_ARTICLE_END", "Finished reading article")
                            except Exception as e:
                                print(f"Error scrolling article: {e}")
                                log_event("ERROR", f"Article scrolling error: {e}")
                            
                            # Go back to main page with error handling
                            print("Going back to main page...")
                            log_event("NAVIGATION_BACK", "Navigating back to CNN homepage")
                            try:
                                driver.back()
                                time.sleep(3)
                                log_event("NAVIGATION_BACK_SUCCESS", "Returned to homepage")
                            except TimeoutException:
                                print("Back navigation timed out, reloading CNN homepage instead...")
                                log_event("NAVIGATION_TIMEOUT", "Back navigation timed out")
                                try:
                                    driver.get("http://www.cnn.com")
                                    time.sleep(3)
                                    log_event("PAGE_LOAD_END", "CNN.com reloaded")
                                except TimeoutException:
                                    print("Reload timed out, but continuing...")
                                    log_event("PAGE_LOAD_TIMEOUT", "Reload timed out")
                                    time.sleep(2)
                            except Exception as e:
                                print(f"Error going back: {e}, reloading instead...")
                                log_event("ERROR", f"Navigation back error: {e}")
                                try:
                                    driver.get("http://www.cnn.com")
                                    time.sleep(3)
                                except:
                                    print("Could not reload, continuing...")
                                    
                        except TimeoutException:
                            print("Article page load timed out, going back...")
                            log_event("PAGE_LOAD_TIMEOUT", "Article page load timed out")
                            try:
                                driver.back()
                                time.sleep(2)
                            except:
                                driver.get("http://www.cnn.com")
                                time.sleep(3)
                                
                        except Exception as e:
                            print(f"Error clicking article: {e}")
                            log_event("ERROR", f"Article click error: {e}")
                            # Try to recover by going back or reloading
                            try:
                                current_url = driver.current_url
                                if 'cnn.com' in current_url and current_url != 'http://www.cnn.com':
                                    driver.back()
                                else:
                                    driver.get("http://www.cnn.com")
                                time.sleep(2)
                            except:
                                print("Could not recover, continuing...")
                                
                    else:
                        print("No valid article links found, scrolling more instead...")
                        log_event("ARTICLE_SEARCH", "No valid articles found")
                        driver.execute_script("window.scrollTo(0, 0);")
                        time.sleep(2)
                        
                except Exception as e:
                    print(f"Error in article selection: {e}")
                    log_event("ERROR", f"Article selection error: {e}")
                    # Try to continue by reloading main page
                    try:
                        driver.get("http://www.cnn.com")
                        time.sleep(3)
                    except:
                        print("Could not reload, continuing...")
                    time.sleep(2)
                
                log_event("ITERATION_END", f"Completed iteration {i+1}/{repetitions}")
            
            print("\nUse Case 1 completed successfully")
            log_event("END", "Use Case 1 completed successfully")
                    
        except Exception as e:
            print(f"Error during Use Case 1: {e}")
            log_event("ERROR", f"Fatal error: {e}")
        
        finally:
            if driver:
                try:
                    log_event("BROWSER_CLOSE", "Closing browser")
                    driver.quit()
                except:
                    print("Error closing browser, forcing close...")
                    log_event("ERROR", "Error closing browser")
            
            # Stop packet capture
            log_event("CAPTURE_STOP", "Stopping packet capture")
            self.stop_packet_capture(capture_process)
            print(f"PCAP file saved: {os.path.join(self.output_dir, pcap_filename)}")
            
            # Save event log to file
            log_filepath = os.path.join(self.output_dir, log_filename)
            try:
                with open(log_filepath, 'w') as f:
                    # Write header
                    f.write("Timestamp,Epoch Time,Event Type,Description\n")
                    # Write events
                    for event in event_log:
                        f.write(f"{event['timestamp']},{event['epoch_time']},{event['event_type']},{event['description']}\n")
                print(f"Event log saved: {log_filepath}")
                
                # Also save as JSON for easier parsing
                json_log_filename = f"usecase1_cnn_browsing_{timestamp}.json"
                json_log_filepath = os.path.join(self.output_dir, json_log_filename)
                with open(json_log_filepath, 'w') as f:
                    json.dump(event_log, f, indent=2)
                print(f"Event log (JSON) saved: {json_log_filepath}")
                
            except Exception as e:
                print(f"Error saving event log: {e}")
    
    def use_case_2_pdf_download(self, repetitions=10, browser_type="firefox"):
        """Use Case 2: Download PDF file multiple times by clicking download button.
        
        Args:
            repetitions (int): Number of times to download the PDF
            browser_type (str): Browser to use
        """
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        pcap_filename = f"usecase2_pdf_download_{timestamp}.pcap"
        log_filename = f"usecase2_pdf_download_{timestamp}.log"
        
        # Initialize event log
        event_log = []
        
        def log_event(event_type, description=""):
            """Helper function to log events with timestamps"""
            event_timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
            event_entry = {
                "timestamp": event_timestamp,
                "epoch_time": time.time(),
                "event_type": event_type,
                "description": description
            }
            event_log.append(event_entry)
            print(f"[{event_timestamp}] {event_type}: {description}")
        
        # Use a page that has a download button/link for the PDF
        # The UMass CS publications page
        page_url = "http://www.ndss-symposium.org/ndss-paper/massbrowser-unblocking-the-censored-web-for-the-masses-by-the-masses/"
        pdf_link_text = "Paper"  # Identifier for the specific PDF
        
        print("\n" + "="*60)
        print("USE CASE 2: PDF Download via Button Click (10 repetitions)")
        print("="*60)
        
        log_event("START", "Use Case 2 started")
        
        # Start packet capture
        log_event("CAPTURE_START", "Starting packet capture")
        capture_process = self.start_packet_capture(pcap_filename)
        
        if not capture_process:
            print("Failed to start packet capture. Aborting use case.")
            log_event("ERROR", "Failed to start packet capture")
            return
        
        driver = None
        try:
            log_event("BROWSER_INIT", f"Initializing {browser_type} browser")
            if browser_type == "firefox":
                options = FirefoxOptions()
                
                # Configure Firefox to auto-download PDFs
                options.set_preference("browser.download.folderList", 2)
                options.set_preference("browser.download.dir", os.path.join(os.getcwd(), "downloads"))
                options.set_preference("browser.download.useDownloadDir", True)
                options.set_preference("browser.helperApps.neverAsk.saveToDisk", "application/pdf")
                options.set_preference("pdfjs.disabled", True)  # Disable PDF viewer
                
                log_event("BROWSER_CONFIG", "Firefox configured for auto-download")
                
                # Configure proxy if specified
                if self.proxy:
                    parsed_proxy = self.proxy.split('://')
                    proxy_protocol = parsed_proxy[0] if len(parsed_proxy) > 1 else 'http'
                    proxy_address = parsed_proxy[-1]
                    
                    if proxy_protocol.startswith('socks'):
                        socks_version = 5 if proxy_protocol in ['socks5', 'socks5h'] else 4
                        host, port = proxy_address.split(':')
                        options.set_preference("network.proxy.type", 1)
                        options.set_preference("network.proxy.socks", host)
                        options.set_preference("network.proxy.socks_port", int(port))
                        options.set_preference("network.proxy.socks_version", socks_version)
                        options.set_preference("network.proxy.socks_remote_dns", proxy_protocol.endswith('h'))
                        log_event("PROXY_CONFIG", f"SOCKS{socks_version} proxy configured: {host}:{port}")
                    else:
                        options.set_preference("network.proxy.type", 1)
                        if proxy_protocol == 'http':
                            host, port = proxy_address.split(':')
                            options.set_preference("network.proxy.http", host)
                            options.set_preference("network.proxy.http_port", int(port))
                            options.set_preference("network.proxy.ssl", host)
                            options.set_preference("network.proxy.ssl_port", int(port))
                            log_event("PROXY_CONFIG", f"HTTP proxy configured: {host}:{port}")
                
                driver = webdriver.Firefox(options=options)
            else:
                options = ChromeOptions()
                prefs = {
                    "download.default_directory": os.path.join(os.getcwd(), "downloads"),
                    "download.prompt_for_download": False,
                    "plugins.always_open_pdf_externally": True
                }
                options.add_experimental_option("prefs", prefs)
                
                if self.proxy:
                    options.add_argument(f'--proxy-server={self.proxy}')
                    log_event("PROXY_CONFIG", f"Proxy configured: {self.proxy}")
                
                driver = webdriver.Chrome(options=options)
                log_event("BROWSER_CONFIG", "Chrome configured for auto-download")
            
            # Set page load timeout to prevent hanging
            driver.set_page_load_timeout(30)
            
            # Create downloads directory
            os.makedirs("downloads", exist_ok=True)
            log_event("SETUP_COMPLETE", "Downloads directory created")
            
            # Navigate to the page once
            print(f"Navigating to publications page: {page_url}")
            log_event("PAGE_LOAD_START", f"Loading {page_url}")
            try:
                driver.get(page_url)
                time.sleep(3)  # Wait for page to load
                log_event("PAGE_LOAD_END", "Publications page loaded successfully")
            except TimeoutException:
                print("Page load timed out, but continuing...")
                log_event("PAGE_LOAD_TIMEOUT", "Initial page load timed out")
                time.sleep(2)
            
            for i in range(repetitions):
                print(f"\nDownload {i+1}/{repetitions}")
                log_event("ITERATION_START", f"Starting download {i+1}/{repetitions}")
                
                try:
                    # Find the download link/button for the PDF
                    # Try multiple selectors to find PDF links
                    pdf_link = None
                    
                    log_event("PDF_SEARCH", "Searching for PDF link")
                    
                    # Method 1: Look for link containing the PDF name
                    try:
                        pdf_link = driver.find_element(By.PARTIAL_LINK_TEXT, pdf_link_text)
                        print(f"Found PDF link by text: {pdf_link_text}")
                        log_event("PDF_FOUND", f"PDF link found by text: {pdf_link_text}")
                    except NoSuchElementException:
                        log_event("PDF_SEARCH", "PDF link not found by text")
                        pass
                    
                    # Method 2: Look for any PDF link
                    if not pdf_link:
                        try:
                            pdf_links = driver.find_elements(By.XPATH, "//a[contains(@href, '.pdf')]")
                            if pdf_links:
                                pdf_link = pdf_links[0]  # Get first PDF link
                                pdf_href = pdf_link.get_attribute('href')
                                print(f"Found PDF link by href: {pdf_href}")
                                log_event("PDF_FOUND", f"PDF link found by href: {pdf_href}")
                        except NoSuchElementException:
                            log_event("PDF_SEARCH", "PDF link not found by href")
                            pass
                    
                    # Method 3: Direct URL approach as fallback
                    if not pdf_link:
                        print("PDF link not found on page, using direct URL...")
                        log_event("PDF_FALLBACK", "Using direct URL fallback")
                        direct_pdf_url = "http://www.ndss-symposium.org/wp-content/uploads/2020/02/24340-paper.pdf"
                        
                        log_event("DOWNLOAD_START", f"Direct download: {direct_pdf_url}")
                        try:
                            driver.get(direct_pdf_url)
                            time.sleep(5)
                            print(f"Downloaded PDF via direct URL")
                            log_event("DOWNLOAD_END", "PDF downloaded via direct URL")
                        except TimeoutException:
                            print("Direct URL download timed out, but file may have been downloaded")
                            log_event("DOWNLOAD_TIMEOUT", "Direct URL download timed out")
                            time.sleep(2)
                        except Exception as e:
                            print(f"Error with direct URL: {e}")
                            log_event("ERROR", f"Direct URL download error: {e}")
                        
                        # Navigate back to the page for next iteration
                        if i < repetitions - 1:
                            log_event("NAVIGATION_BACK", "Returning to publications page")
                            try:
                                driver.get(page_url)
                                time.sleep(3)
                                log_event("NAVIGATION_BACK_SUCCESS", "Returned to publications page")
                            except TimeoutException:
                                print("Page reload timed out, but continuing...")
                                log_event("NAVIGATION_TIMEOUT", "Page reload timed out")
                                time.sleep(2)
                    else:
                        # Click the download link/button
                        print("Clicking download link...")
                        
                        try:
                            # Get the PDF URL before clicking
                            pdf_url = pdf_link.get_attribute('href')
                            log_event("DOWNLOAD_START", f"Clicking PDF link: {pdf_url}")
                            
                            # Scroll the link into view
                            driver.execute_script("arguments[0].scrollIntoView(true);", pdf_link)
                            time.sleep(1)
                            
                            # Click the link
                            pdf_link.click()
                            log_event("PDF_CLICK", "PDF link clicked")
                            
                            # Wait for download to initiate and complete
                            time.sleep(5)
                            
                            print(f"Download {i+1} completed via button click")
                            log_event("DOWNLOAD_END", f"Download {i+1} completed")
                            
                        except TimeoutException:
                            print("Click/download timed out, but download may have started")
                            log_event("DOWNLOAD_TIMEOUT", "Click/download timed out")
                            time.sleep(2)
                        except Exception as e:
                            print(f"Error clicking link: {e}")
                            log_event("ERROR", f"Click error: {e}")
                            # Try direct navigation as fallback
                            try:
                                if pdf_url:
                                    print(f"Trying direct navigation to: {pdf_url}")
                                    log_event("DOWNLOAD_FALLBACK", f"Direct navigation to: {pdf_url}")
                                    driver.get(pdf_url)
                                    time.sleep(5)
                                    log_event("DOWNLOAD_END", "Download completed via fallback")
                            except TimeoutException:
                                print("Fallback download timed out")
                                log_event("DOWNLOAD_TIMEOUT", "Fallback download timed out")
                                time.sleep(2)
                            except:
                                pass
                        
                        # Navigate back to the page for next iteration
                        if i < repetitions - 1:
                            try:
                                print("Navigating back to list page...")
                                log_event("NAVIGATION_BACK", "Returning to publications page")
                                driver.get(page_url)
                                time.sleep(3)
                                log_event("NAVIGATION_BACK_SUCCESS", "Returned to publications page")
                            except TimeoutException:
                                print("Back navigation timed out, but continuing...")
                                log_event("NAVIGATION_TIMEOUT", "Back navigation timed out")
                                time.sleep(2)
                            except Exception as e:
                                print(f"Error navigating back: {e}")
                                log_event("ERROR", f"Navigation back error: {e}")
                                time.sleep(2)
                    
                except Exception as e:
                    print(f"Error during download {i+1}: {e}")
                    log_event("ERROR", f"Download {i+1} error: {e}")
                    print("Attempting to continue...")
                    
                    # Try to recover by reloading the page
                    try:
                        log_event("RECOVERY", "Attempting to reload page")
                        driver.get(page_url)
                        time.sleep(3)
                        log_event("RECOVERY_SUCCESS", "Page reloaded successfully")
                    except TimeoutException:
                        print("Recovery reload timed out")
                        log_event("RECOVERY_TIMEOUT", "Recovery reload timed out")
                        time.sleep(2)
                    except:
                        pass
                
                log_event("ITERATION_END", f"Completed download iteration {i+1}/{repetitions}")
                
                # Small delay between downloads
                if i < repetitions - 1:
                    time.sleep(2)
            
            print("\nUse Case 2 completed successfully")
            log_event("END", "Use Case 2 completed successfully")
            
        except Exception as e:
            print(f"Error during Use Case 2: {e}")
            log_event("ERROR", f"Fatal error: {e}")
        
        finally:
            if driver:
                try:
                    log_event("BROWSER_CLOSE", "Closing browser")
                    driver.quit()
                except:
                    print("Error closing browser, forcing close...")
                    log_event("ERROR", "Error closing browser")
            
            # Stop packet capture
            log_event("CAPTURE_STOP", "Stopping packet capture")
            self.stop_packet_capture(capture_process)
            print(f"PCAP file saved: {os.path.join(self.output_dir, pcap_filename)}")
            
            # Save event log to file
            log_filepath = os.path.join(self.output_dir, log_filename)
            try:
                with open(log_filepath, 'w') as f:
                    # Write header
                    f.write("Timestamp,Epoch Time,Event Type,Description\n")
                    # Write events
                    for event in event_log:
                        f.write(f"{event['timestamp']},{event['epoch_time']},{event['event_type']},{event['description']}\n")
                print(f"Event log saved: {log_filepath}")
                
                # Also save as JSON for easier parsing
                json_log_filename = f"usecase2_pdf_download_{timestamp}.json"
                json_log_filepath = os.path.join(self.output_dir, json_log_filename)
                with open(json_log_filepath, 'w') as f:
                    json.dump(event_log, f, indent=2)
                print(f"Event log (JSON) saved: {json_log_filepath}")
                
            except Exception as e:
                print(f"Error saving event log: {e}")
    
    def use_case_3_video_streaming(self, repetitions=5, browser_type="firefox"):
        """Use Case 3: Watch short streamed video multiple times.
        
        Args:
            repetitions (int): Number of times to watch the video
            browser_type (str): Browser to use
        """
        timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
        pcap_filename = f"usecase3_video_streaming_{timestamp}.pcap"
        log_filename = f"usecase3_video_streaming_{timestamp}.log"
        video_url = "http://www.videezy.com/people/3544-daytime-times-square-low-level-shot-4k"
        
        # Initialize event log
        event_log = []
        
        def log_event(event_type, description=""):
            """Helper function to log events with timestamps"""
            event_timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
            event_entry = {
                "timestamp": event_timestamp,
                "epoch_time": time.time(),
                "event_type": event_type,
                "description": description
            }
            event_log.append(event_entry)
            print(f"[{event_timestamp}] {event_type}: {description}")
        
        print("\n" + "="*60)
        print("USE CASE 3: Video Streaming (5 repetitions)")
        print("="*60)
        
        log_event("START", "Use Case 3 started")
        
        # Start packet capture
        log_event("CAPTURE_START", "Starting packet capture")
        capture_process = self.start_packet_capture(pcap_filename)
        
        if not capture_process:
            print("Failed to start packet capture. Aborting use case.")
            log_event("ERROR", "Failed to start packet capture")
            return
        
        driver = None
        try:
            log_event("BROWSER_INIT", f"Initializing {browser_type} browser")
            driver = self.setup_browser(browser_type)
            driver.maximize_window()
            log_event("BROWSER_READY", "Browser maximized and ready")
            
            # Set page load timeout to prevent hanging
            driver.set_page_load_timeout(30)
            
            for i in range(repetitions):
                print(f"\n--- Playback {i+1}/{repetitions} ---")
                log_event("ITERATION_START", f"Starting playback {i+1}/{repetitions}")
                
                print(f"Loading video page: {video_url}")
                log_event("PAGE_LOAD_START", f"Loading video page: {video_url}")
                
                try:
                    driver.get(video_url)
                    time.sleep(1)  # Wait for page to load
                    log_event("PAGE_LOAD_END", "Video page loaded successfully")
                except TimeoutException:
                    print("Page load timed out, but continuing...")
                    log_event("PAGE_LOAD_TIMEOUT", "Video page load timed out")
                    time.sleep(1)
                
                # Wait for any redirects to complete
                current_url = driver.current_url
                print(f"Current URL: {current_url}")
                log_event("PAGE_READY", f"Current URL: {current_url}")
                time.sleep(1)
                
                # Try to find and play video
                try:
                    # Look for video element
                    print("Looking for video element...")
                    log_event("VIDEO_SEARCH", "Searching for video element")
                    
                    # Wait for video to be present
                    video = WebDriverWait(driver, 10).until(
                        EC.presence_of_element_located((By.TAG_NAME, "video"))
                    )
                    
                    print("Video element found, attempting to play...")
                    log_event("VIDEO_FOUND", "Video element found")
                    
                    # Try to click play button if exists
                    try:
                        play_button = driver.find_element(By.CSS_SELECTOR, "button[aria-label*='play'], .play-button, button.play")
                        play_button.click()
                        print("Clicked play button")
                        log_event("VIDEO_PLAY", "Play button clicked")
                    except:
                        # Try to play video directly with JavaScript
                        driver.execute_script("arguments[0].play();", video)
                        print("Started video playback via JavaScript")
                        log_event("VIDEO_PLAY", "Video started via JavaScript")
                    
                    # Watch for a short duration (adjust based on video length)
                    watch_duration = 11  # seconds
                    print(f"Watching video for {watch_duration} seconds...")
                    log_event("VIDEO_WATCHING", f"Watching video for {watch_duration} seconds")
                    time.sleep(watch_duration)
                    
                    # Pause the video
                    try:
                        driver.execute_script("arguments[0].pause();", video)
                        print("Paused video")
                        log_event("VIDEO_PAUSE", "Video paused")
                    except:
                        log_event("VIDEO_PAUSE", "Could not pause video")
                        pass
                    
                    log_event("VIDEO_COMPLETE", f"Video playback {i+1} completed")
                    
                except TimeoutException:
                    print("Video element not found, page may have redirected or video may not be available")
                    log_event("VIDEO_NOT_FOUND", "Video element not found (timeout)")
                    print("Waiting for page content to load anyway...")
                    log_event("VIDEO_WAIT", "Waiting for content to load")
                    time.sleep(15)
                except Exception as e:
                    print(f"Error playing video: {e}")
                    log_event("ERROR", f"Video playback error: {e}")
                    print("Continuing with next iteration...")
                    time.sleep(10)
                
                log_event("ITERATION_END", f"Completed playback iteration {i+1}/{repetitions}")
                
                # Delay before next repetition
                if i < repetitions - 1:
                    print("Waiting before next playback...")
                    log_event("WAIT", "Delay before next iteration")
                    time.sleep(1)
            
            print("\nUse Case 3 completed successfully")
            log_event("END", "Use Case 3 completed successfully")
            
        except Exception as e:
            print(f"Error during Use Case 3: {e}")
            log_event("ERROR", f"Fatal error: {e}")
        
        finally:
            if driver:
                try:
                    log_event("BROWSER_CLOSE", "Closing browser")
                    driver.quit()
                except:
                    print("Error closing browser, forcing close...")
                    log_event("ERROR", "Error closing browser")
            
            # Stop packet capture
            log_event("CAPTURE_STOP", "Stopping packet capture")
            self.stop_packet_capture(capture_process)
            print(f"PCAP file saved: {os.path.join(self.output_dir, pcap_filename)}")
            
            # Save event log to file
            log_filepath = os.path.join(self.output_dir, log_filename)
            try:
                with open(log_filepath, 'w') as f:
                    # Write header
                    f.write("Timestamp,Epoch Time,Event Type,Description\n")
                    # Write events
                    for event in event_log:
                        f.write(f"{event['timestamp']},{event['epoch_time']},{event['event_type']},{event['description']}\n")
                print(f"Event log saved: {log_filepath}")
                
                # Also save as JSON for easier parsing
                json_log_filename = f"usecase3_video_streaming_{timestamp}.json"
                json_log_filepath = os.path.join(self.output_dir, json_log_filename)
                with open(json_log_filepath, 'w') as f:
                    json.dump(event_log, f, indent=2)
                print(f"Event log (JSON) saved: {json_log_filepath}")
                
            except Exception as e:
                print(f"Error saving event log: {e}")


def main():
    # Set default interface based on platform
    default_interface = "any" if sys.platform != 'win32' else "1"  # Windows uses interface numbers
    
    parser = argparse.ArgumentParser(description='Capture PCAP files for different use cases')
    parser.add_argument('--use-case', type=int, choices=[1, 2, 3], 
                        help='Use case to run (1: CNN browsing, 2: PDF download, 3: Video streaming)')
    parser.add_argument('--all', action='store_true', help='Run all use cases')
    parser.add_argument('--output-dir', default='pcap_captures', help='Directory for PCAP files')
    parser.add_argument('--interface', default=default_interface, 
                        help=f'Network interface to capture (default: {default_interface})')
    parser.add_argument('--browser', choices=['firefox', 'chrome'], default='firefox', 
                        help='Browser to use (default: firefox)')
    parser.add_argument('--proxy', help='Proxy server (format: protocol://host:port)')
    parser.add_argument('--proxy-user', help='Proxy username')
    parser.add_argument('--proxy-pass', help='Proxy password')
    
    args = parser.parse_args()
    
    if not args.use_case and not args.all:
        parser.error("Either --use-case or --all must be specified")
    
    # Validate proxy authentication
    if args.proxy_user or args.proxy_pass:
        if not args.proxy:
            parser.error("--proxy-user and --proxy-pass require --proxy")
        if bool(args.proxy_user) != bool(args.proxy_pass):
            parser.error("Both --proxy-user and --proxy-pass must be provided")
    
    # Initialize tester
    tester = PcapCaptureTester(
        output_dir=args.output_dir,
        interface=args.interface,
        proxy=args.proxy,
        proxy_user=args.proxy_user,
        proxy_pass=args.proxy_pass
    )
    
    # Run use cases
    if args.all or args.use_case == 1:
        tester.use_case_1_cnn_browsing(repetitions=5, browser_type=args.browser)
    
    if args.all or args.use_case == 2:
        tester.use_case_2_pdf_download(repetitions=10, browser_type=args.browser)
    
    if args.all or args.use_case == 3:
        tester.use_case_3_video_streaming(repetitions=5, browser_type=args.browser)
    
    print("\n" + "="*60)
    print("All requested use cases completed!")
    print(f"PCAP files saved in: {args.output_dir}")
    print("="*60)


if __name__ == "__main__":
    main()
