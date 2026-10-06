# Migrations


We need some way how to migrate data when structure is changed, it must be completed before app launches itself, but previous version should be running, maybe not using migrated content in that time, its ok


We need ability to migrate entities to other projects, entity will be copied to other project and some way labeled from where it should be migrated, and target project will copy all data.

## Seeds

Ability to mark what is test data, what is production data, and prevent write data twice.


I am not sure here: Fix ids, seeds should be possible to run event when ids are there? Maybe its bad behavior.