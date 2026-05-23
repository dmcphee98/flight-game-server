
# Welcome to your CDK Python project!

This is a blank project for CDK development with Python.

The `cdk.json` file tells the CDK Toolkit how to execute your app.

This project is set up like a standard Python project.  The initialization
process also creates a virtualenv within this project, stored under the `.venv`
directory.  To create the virtualenv it assumes that there is a `python3`
(or `python` for Windows) executable in your path with access to the `venv`
package. If for any reason the automatic creation of the virtualenv fails,
you can create the virtualenv manually.

To manually create a virtualenv on MacOS and Linux:

```
$ python -m venv .venv
```

After the init process completes and the virtualenv is created, you can use the following
step to activate your virtualenv.

```
$ source .venv/bin/activate
```

If you are a Windows platform, you would activate the virtualenv like this:

```
% .venv\Scripts\activate.bat
```

Once the virtualenv is activated, you can install the required dependencies.

```
$ pip install -r requirements.txt
```
Before synthesizing or deploying, sign in to AWS via the CLI:

```
$ aws login
```

The CDK stack pulls the AWS account ID and region from your active session. You can get/set your currently configured region with:

```
$ aws configure get region
$ aws configure set region ap-southeast-2
```


At this point you can now synthesize the CloudFormation template for this code.

```
$ cdk synth
-c domain=example.com 
-c subdomain=game 
-c port=8080
```

To deploy the stack to your default AWS account/region:

```
$ cdk deploy
-c domain=example.com 
-c subdomain=game 
-c port=8080
```



To add additional dependencies, for example other CDK libraries, just add
them to your `requirements.txt` file and rerun the `python -m pip install -r requirements.txt`
command.

## Useful commands

 * `cdk ls`          list all stacks in the app
 * `cdk diff`        compare deployed stack with current state
 * `cdk docs`        open CDK documentation

Enjoy!
