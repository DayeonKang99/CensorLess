import uuid
from django.db import models
from django.core.exceptions import ValidationError
from django.contrib.auth.models import (
    PermissionsMixin,
)
import requests
from scripts.config_basic import WORLD_SIZE, CENSORED_REGION_SIZE
from random import random


def get_func_url_location(function_url: str):
    # base_url = f"http://ipapi.co/{ip}/json/"
    # response = requests.get(base_url)
    # location = response.json()
    # return location["latitude"], location["longitude"]

    location = function_url.split('.')[2]
    return location


class ProxyManager(models.Manager):
    def create(self, **kwargs):
        proxy_url = kwargs.get("function_url", None)
        is_test = kwargs.get("is_test", None)
        ## why do this thing?
        while True:
            latitude = (random() * WORLD_SIZE) - (WORLD_SIZE // 2)
            if latitude < -CENSORED_REGION_SIZE or latitude > CENSORED_REGION_SIZE:
                break
        kwargs["latitude"] = latitude

        while True:
            longitude = (random() * WORLD_SIZE) - (WORLD_SIZE // 2)
            if longitude < -CENSORED_REGION_SIZE or longitude > CENSORED_REGION_SIZE:
                break
        kwargs["longitude"] = longitude

        instance = super().create(**kwargs)
        return instance


class Proxy(models.Model):
    # url = models.CharField(max_length=100, null=True)
    function_url = models.CharField(max_length=100, null=False, default="")
    is_test = models.BooleanField(default=True)
    created_at = models.DateTimeField(auto_now_add=True)
    capacity = models.IntegerField(default=40)
    is_blocked = models.BooleanField(default=False)
    blocked_at = models.IntegerField(default=0)
    is_active = models.BooleanField(default=True)
    deactivated_at = models.IntegerField(default=0)
    latitude = models.FloatField(null=True)
    longitude = models.FloatField(null=True)

    objects = ProxyManager()

    def __str__(self):
        return str(self.function_url)


class ClientManager(models.Manager):
    def create(self, **kwargs):
        user_ip = kwargs.get("ip", None)
        is_test = kwargs.get("is_test", None)

        kwargs["latitude"] = (
            random() * CENSORED_REGION_SIZE * 2
        ) - CENSORED_REGION_SIZE
        kwargs["longitude"] = (
            random() * CENSORED_REGION_SIZE * 2
        ) - CENSORED_REGION_SIZE

        instance = super().create(**kwargs)

        return instance


class Client(models.Model):
    ip = models.CharField(max_length=30, null=False, unique=True, primary_key=True)
    is_test = models.BooleanField(default=True)
    is_censor_agent = models.BooleanField(default=False)
    flagged = models.BooleanField(default=False)
    user_agent = models.CharField(max_length=255, null=True, blank=True)
    latitude = models.FloatField(null=True)
    longitude = models.FloatField(null=True)
    request_count = models.IntegerField(default=0)
    known_blocked_proxies = models.IntegerField(default=0)
    creation_time = models.IntegerField(default=0)

    objects = ClientManager()


class ProxyReport(models.Model):
    uuid = models.UUIDField(
        default=uuid.uuid4, editable=False, unique=True, primary_key=True
    )
    proxy = models.ForeignKey(
        Proxy,
        null=True,
        blank=True,
        on_delete=models.CASCADE,
        related_name="reports_given",
    )
    created_at = models.DateTimeField(auto_now_add=True)
    utility = models.FloatField()
    throughput = models.FloatField()
    connected_clients = models.ManyToManyField(
        Client, related_name="proxies_connected", blank=True
    )


class Assignment(models.Model):
    proxy = models.ForeignKey(
        Proxy,
        null=True,
        blank=True,
        on_delete=models.CASCADE,
        related_name="assignee",
    )
    client = models.ForeignKey(
        Client,
        null=True,
        blank=True,
        on_delete=models.CASCADE,
        related_name="assigned",
    )
    created_at = models.DateTimeField(auto_now_add=True)
    assignment_time = models.IntegerField(null=False, default=0)
    from_migration = models.BooleanField(default=False)
    is_expired = models.BooleanField(default=False)


class IDClientCounter(models.Model):
    created_at = models.DateTimeField(auto_now_add=True)


class ClientAvgMigrationTime(models.Model):
    value = models.FloatField()
    client_ip = models.CharField(max_length=30)
    created_at = models.DateTimeField(auto_now_add=True)


class ProxyAvgMigrationTime(models.Model):
    value = models.FloatField()
    proxy_url = models.CharField(max_length=100)
    created_at = models.DateTimeField(auto_now_add=True)


class ControllerAvgMigrationTime(models.Model):
    value = models.FloatField()
    created_at = models.DateTimeField(auto_now_add=True)


class ChartNonBlockedProxyRatio(models.Model):
    value = models.FloatField()
    creation_time = models.IntegerField()


class ChartNonBlockedProxyCount(models.Model):
    value = models.IntegerField()
    creation_time = models.IntegerField()


class ChartConnectedUsersRatio(models.Model):
    value = models.FloatField()
    creation_time = models.IntegerField()
